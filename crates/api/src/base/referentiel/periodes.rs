//! Transitions des périodes et ouverture de l'exercice suivant (L2-23, PR 11b ; migration
//! `0201_periodes.sql` ; note d'étude de la PR 11, §6).
//!
//! Chaque transition suit le même chemin : verrous, **relecture** des exercices
//! (`Exercices::reconstituer`), transition **pure** de `compta-core` sur une copie, puis écriture
//! des **seules lignes changées**. Rien n'est écrit si la transition est refusée.
//!
//! # Ordre des verrous (ADR 0011)
//! 1. référentiel du dossier ([`super::verrouiller`]), seulement pour l'ouverture de l'exercice
//!    suivant : deux ouvertures concurrentes se suivent au lieu de se heurter sur l'unicité du
//!    code ;
//! 2. périodes, **dans l'ordre chronologique** : pour une clôture, les périodes antérieures
//!    `FOR SHARE`, puis la période visée `FOR UPDATE` ; pour un verrouillage ou un
//!    déverrouillage, la seule période visée `FOR UPDATE`. Une clôture concurrente de la période
//!    précédente est ainsi **attendue** (au lieu d'un refus à tort), une validation en cours (qui
//!    tient sa période `FOR SHARE`, PR 13) retarde le verrouillage, et l'ordre unique exclut
//!    l'interblocage ;
//! 3. têtes d'audit, prises ensuite par le pipeline.
//!
//! Une ligne clôturée reste visible et verrouillable ; toute modification en est refusée par
//! un déclencheur (`SB001`, migration `0201`), seconde barrière après `compta-core`. L'état est
//! relu par une lecture simple après les verrous.
//!
//! # Brouillard
//! La clôture d'une période est refusée tant qu'une écriture du brouillard y est datée (#135).
//! Les dates sont lues **après** le verrou de la période ([`dates_du_brouillard`]). Aucune table
//! de brouillard n'existe avant la PR 13 : la liste vide est alors exacte. Fil-piège : toute table
//! dont le nom contient `brouill` doit figurer dans [`TABLES_BROUILLARD_LUES_A_LA_CLOTURE`] (test de catalogue).

use std::collections::BTreeMap;

use compta_core::{CodeExercice, ErreurExercice, Exercice, Exercices, StatutPeriode, TexteCourt};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{
    ErreurBase, TransactionTenant, charger_exercices, contexte, nom_statut_exercice,
    nom_statut_periode, verrouiller,
};

/// Tables du brouillard dont [`dates_du_brouillard`] lit les dates à la clôture d'une période.
///
/// **Fil-piège** : le test de catalogue (`tests/catalogue.rs`) exige que cette liste, les tables
/// de la base dont le nom contient `brouill` et la liste `[[brouillard]]` du catalogue soient le même ensemble. La
/// PR 13 ne peut donc pas créer sa table sans brancher sa lecture ici (et sans prendre la période
/// `FOR SHARE` à la création et au changement de date d'un brouillon).
pub const TABLES_BROUILLARD_LUES_A_LA_CLOTURE: &[&str] = &[];

/// Date comptable de chaque écriture du brouillard du dossier du contexte datée dans
/// `[du, au]`, une par écriture : la lecture de **chaque** table de
/// [`TABLES_BROUILLARD_LUES_A_LA_CLOTURE`]. À appeler **après** le verrou `FOR UPDATE` de la
/// période.
///
/// Aucune table de brouillard n'existe encore : la liste vide est exacte.
///
/// # Errors
/// Contexte sans dossier ; table de la liste sans lecture branchée ([`lire_dates`]) ; erreur de
/// PostgreSQL.
pub async fn dates_du_brouillard(
    tx: &mut TransactionTenant,
    du: Date,
    au: Date,
) -> Result<Vec<Date>, ErreurBase> {
    contexte(tx)?;
    let mut dates = Vec::new();
    for table in TABLES_BROUILLARD_LUES_A_LA_CLOTURE {
        dates.extend(lire_dates(tx, table, du, au).await?);
    }
    Ok(dates)
}

/// Dates des écritures d'une table du brouillard datées dans `[du, au]`. La PR 13 y branche une
/// requête `sqlx::query!` par table ; une table de [`TABLES_BROUILLARD_LUES_A_LA_CLOTURE`] sans
/// lecture branchée est une erreur : toute clôture échoue alors, et les tests avec elle
/// (`tests/periodes.rs`, `chaque_table_du_brouillard_a_sa_lecture`).
#[allow(clippy::unused_async)] // Lira la base à la PR 13.
async fn lire_dates(
    tx: &mut TransactionTenant,
    table: &str,
    du: Date,
    au: Date,
) -> Result<Vec<Date>, ErreurBase> {
    let _ = (tx, table, du, au);
    Err(ErreurBase::Donnee(
        "table du brouillard sans lecture branchée à la clôture",
    ))
}

/// Transition d'une période.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// Ouverte → verrouillée.
    Verrouiller,
    /// Verrouillée → ouverte, avec son motif (journalisé).
    Deverrouiller(TexteCourt),
    /// Clôture informatique (AUDCIF art. 22 3°), irréversible.
    Cloturer,
}

/// Motif stable du refus d'une transition ou d'une ouverture par les règles de `compta-core`
/// (réponse 409 de l'API, un code par motif).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefusPeriode {
    /// La période est déjà clôturée (quelle que soit la transition demandée).
    #[error("période déjà clôturée")]
    PeriodeDejaCloturee,
    /// Verrouillage d'une période qui n'est pas ouverte.
    #[error("période non ouverte")]
    PeriodeNonOuverte,
    /// Déverrouillage d'une période qui n'est pas verrouillée.
    #[error("période non verrouillée")]
    PeriodeNonVerrouillee,
    /// Dernière période d'un exercice : elle se clôture avec lui.
    #[error("dernière période de l'exercice")]
    DernierePeriode,
    /// Une période antérieure n'est pas clôturée (AUDCIF art. 22 3°).
    #[error("période antérieure non clôturée")]
    PeriodeAnterieureNonCloturee,
    /// Écritures du brouillard datées dans la période (#135).
    #[error("{nombre} écriture(s) du brouillard dans la période")]
    BrouillardNonValide {
        /// Nombre d'écritures du brouillard concernées.
        nombre: usize,
    },
    /// Deux exercices sont déjà non clôturés.
    #[error("deux exercices non clôturés au plus")]
    ExercicesNonCloturesMax,
    /// Autre refus des règles.
    #[error(transparent)]
    Autre(ErreurExercice),
}

/// Échec d'une transition ou d'une ouverture.
#[derive(Debug, thiserror::Error)]
pub enum ErreurPeriode {
    /// Période absente du dossier du contexte (ou d'autrui), ou dossier sans référentiel.
    #[error("période ou référentiel absent")]
    Absente,
    /// Transition refusée par les règles de `compta-core`.
    #[error(transparent)]
    Refus(#[from] RefusPeriode),
    /// Erreur d'accès à la base.
    #[error(transparent)]
    Base(#[from] ErreurBase),
}

impl From<sqlx::Error> for ErreurPeriode {
    fn from(erreur: sqlx::Error) -> Self {
        Self::Base(ErreurBase::Sqlx(erreur))
    }
}

/// Changement de statut enregistré d'une période.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangementPeriode {
    /// Identifiant de la période.
    pub id: Uuid,
    /// Exercice de la période.
    pub exercice: CodeExercice,
    /// Premier jour.
    pub debut: Date,
    /// Statut avant.
    pub avant: StatutPeriode,
    /// Statut après.
    pub apres: StatutPeriode,
    /// Motif d'un déverrouillage.
    pub motif: Option<TexteCourt>,
    /// Exercices après la transition (pour la réponse).
    pub exercices: Exercices,
}

/// Applique une transition à la période `periode` du dossier du contexte, au nom de `auteur` à
/// l'instant `instant` (voir le module pour les verrous et l'ordre des étapes).
///
/// # Errors
/// [`ErreurPeriode`] : période absente, transition refusée, erreur de base.
pub async fn changer_statut(
    tx: &mut TransactionTenant,
    periode: Uuid,
    transition: Transition,
    auteur: Uuid,
    instant: OffsetDateTime,
) -> Result<ChangementPeriode, ErreurPeriode> {
    let (cabinet, dossier) = contexte(tx)?;
    let debut = sqlx::query_scalar!(
        "SELECT debut FROM periode WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet,
        dossier,
        periode,
    )
    .fetch_optional(tx.connexion())
    .await?
    .ok_or(ErreurPeriode::Absente)?;

    // Verrous, dans l'ordre chronologique.
    if transition == Transition::Cloturer {
        sqlx::query_scalar!(
            "SELECT id FROM periode WHERE cabinet_id = $1 AND dossier_id = $2 AND debut < $3 \
             ORDER BY debut FOR SHARE",
            cabinet,
            dossier,
            debut,
        )
        .fetch_all(tx.connexion())
        .await?;
    }
    sqlx::query_scalar!(
        "SELECT id FROM periode WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 \
         FOR UPDATE",
        cabinet,
        dossier,
        periode,
    )
    .fetch_optional(tx.connexion())
    .await?;

    // Relecture, après les verrous : l'état committé le plus récent ; puis le brouillard (pour
    // une clôture), puis la transition pure.
    let (relus, _, ids_periodes) = charger_exercices(tx, cabinet, dossier).await?;
    let brouillard = if transition == Transition::Cloturer {
        let fin = relus
            .exercice_contenant(debut)
            .and_then(|e| e.periode_contenant(debut))
            .map(compta_core::Periode::fin)
            .ok_or(ErreurPeriode::Absente)?;
        dates_du_brouillard(tx, debut, fin).await?
    } else {
        Vec::new()
    };
    let (apres, motif) = appliquer(&relus, debut, transition, brouillard)?;

    // Écriture des seules lignes changées : la période visée, et elle seule.
    let (_, avant_lignes) = relus.aplatir();
    let (_, apres_lignes) = apres.aplatir();
    let changees: Vec<_> = avant_lignes
        .iter()
        .zip(&apres_lignes)
        .filter(|(a, b)| a != b)
        .collect();
    let [(a, b)] = changees.as_slice() else {
        return Err(ErreurBase::Donnee("transition de période : changement inattendu").into());
    };
    if a.debut != debut || ids_periodes.get(&debut) != Some(&periode) {
        return Err(ErreurBase::Donnee("transition de période : autre période changée").into());
    }
    let ecrites = sqlx::query!(
        "UPDATE periode SET statut = $5, statut_par = $6, statut_le = $7, motif = $8 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 AND statut = $4",
        cabinet,
        dossier,
        periode,
        nom_statut_periode(a.statut),
        nom_statut_periode(b.statut),
        auteur,
        instant,
        motif.as_ref().map(TexteCourt::as_str),
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    if ecrites != 1 {
        return Err(ErreurBase::Donnee("transition de période : ligne non écrite").into());
    }
    Ok(ChangementPeriode {
        id: periode,
        exercice: a.exercice.clone(),
        debut,
        avant: a.statut,
        apres: b.statut,
        motif,
        exercices: apres,
    })
}

/// Étape pure de [`changer_statut`] : la transition de `compta-core` appliquée à une copie de
/// l'état relu, avec les dates du `brouillard` lues après les verrous ; rend l'état après et le
/// motif d'un déverrouillage, ou le motif stable du refus.
///
/// # Errors
/// [`RefusPeriode`].
pub fn appliquer(
    relus: &Exercices,
    debut: Date,
    transition: Transition,
    brouillard: Vec<Date>,
) -> Result<(Exercices, Option<TexteCourt>), RefusPeriode> {
    let deja_cloturee = relus
        .exercice_contenant(debut)
        .and_then(|e| e.periode_contenant(debut))
        .is_some_and(|p| p.statut() == StatutPeriode::Cloturee);
    let interdite = match &transition {
        Transition::Verrouiller => RefusPeriode::PeriodeNonOuverte,
        Transition::Deverrouiller(_) => RefusPeriode::PeriodeNonVerrouillee,
        // Une période non clôturée ne se voit refuser sa clôture par `TransitionInterdite` que
        // si elle est la dernière de son exercice.
        Transition::Cloturer => RefusPeriode::DernierePeriode,
    };
    let motif_du_refus = |erreur: ErreurExercice| {
        if deja_cloturee {
            return RefusPeriode::PeriodeDejaCloturee;
        }
        match erreur {
            ErreurExercice::TransitionInterdite(_) => interdite.clone(),
            ErreurExercice::PeriodeAnterieureNonCloturee(_) => {
                RefusPeriode::PeriodeAnterieureNonCloturee
            }
            ErreurExercice::BrouillardNonValide { nombre, .. } => {
                RefusPeriode::BrouillardNonValide { nombre }
            }
            autre => RefusPeriode::Autre(autre),
        }
    };
    let mut apres = relus.clone();
    let motif = match transition {
        Transition::Verrouiller => {
            apres.verrouiller_periode(debut).map_err(motif_du_refus)?;
            None
        }
        Transition::Deverrouiller(motif) => Some(
            apres
                .deverrouiller_periode(debut, motif)
                .map_err(motif_du_refus)?
                .motif,
        ),
        Transition::Cloturer => {
            apres
                .cloturer_periode(debut, brouillard)
                .map_err(motif_du_refus)?;
            None
        }
    };
    Ok((apres, motif))
}

/// Exercice ouvert par [`ouvrir_exercice_suivant`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExerciceOuvert {
    /// Identifiant de l'exercice.
    pub id: Uuid,
    /// L'exercice, avec ses périodes.
    pub exercice: Exercice,
    /// Identifiants de ses périodes, par premier jour.
    pub ids_periodes: BTreeMap<Date, Uuid>,
}

/// Ouvre l'exercice suivant du dossier du contexte (`Exercices::ouvrir_suivant`) : l'année
/// civile qui suit la clôture du dernier, au plus deux exercices non clôturés.
///
/// # Errors
/// [`ErreurPeriode`] : dossier sans référentiel, ouverture refusée, erreur de base.
pub async fn ouvrir_exercice_suivant(
    tx: &mut TransactionTenant,
) -> Result<ExerciceOuvert, ErreurPeriode> {
    if !verrouiller(tx).await? {
        return Err(ErreurPeriode::Absente);
    }
    let (cabinet, dossier) = contexte(tx)?;
    let (mut exercices, ids, _) = charger_exercices(tx, cabinet, dossier).await?;
    if ids.is_empty() {
        return Err(ErreurPeriode::Absente);
    }
    let exercice = exercices
        .ouvrir_suivant()
        .map_err(|e| match e {
            ErreurExercice::TropDExercicesOuverts => RefusPeriode::ExercicesNonCloturesMax,
            autre => RefusPeriode::Autre(autre),
        })?
        .clone();
    let (id, ids_periodes) = inserer_exercice(tx, &exercice).await?;
    Ok(ExerciceOuvert {
        id,
        exercice,
        ids_periodes,
    })
}

/// Inscrit un exercice et ses périodes ; rend leurs identifiants.
pub(super) async fn inserer_exercice(
    tx: &mut TransactionTenant,
    exercice: &Exercice,
) -> Result<(Uuid, BTreeMap<Date, Uuid>), ErreurBase> {
    let (cabinet, dossier) = contexte(tx)?;
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO exercice (cabinet_id, dossier_id, id, code, debut, fin, statut) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        cabinet,
        dossier,
        id,
        exercice.code().as_str(),
        exercice.debut(),
        exercice.fin(),
        nom_statut_exercice(exercice.statut()),
    )
    .execute(tx.connexion())
    .await?;
    let periodes = exercice.periodes();
    let ids: Vec<Uuid> = periodes.iter().map(|_| Uuid::new_v4()).collect();
    let exercices = vec![id; periodes.len()];
    let debuts: Vec<Date> = periodes.iter().map(compta_core::Periode::debut).collect();
    let ids_periodes: BTreeMap<Date, Uuid> =
        debuts.iter().copied().zip(ids.iter().copied()).collect();
    let fins: Vec<Date> = periodes.iter().map(compta_core::Periode::fin).collect();
    let statuts: Vec<String> = periodes
        .iter()
        .map(|p| nom_statut_periode(p.statut()).to_owned())
        .collect();
    sqlx::query!(
        "INSERT INTO periode (cabinet_id, dossier_id, id, exercice_id, debut, fin, statut) \
         SELECT $1, $2, p.id, p.exercice, p.debut, p.fin, p.statut \
         FROM UNNEST($3::uuid[], $4::uuid[], $5::date[], $6::date[], $7::text[]) \
              AS p (id, exercice, debut, fin, statut)",
        cabinet,
        dossier,
        &ids,
        &exercices,
        &debuts,
        &fins,
        &statuts,
    )
    .execute(tx.connexion())
    .await?;
    Ok((id, ids_periodes))
}

#[cfg(test)]
mod tests {
    use compta_core::{Exercices, TexteCourt};
    use time::{Date, Month};

    use super::{RefusPeriode, TABLES_BROUILLARD_LUES_A_LA_CLOTURE, Transition, appliquer};

    fn jour(mois: Month, jour: u8) -> Date {
        Date::from_calendar_date(2026, mois, jour).unwrap()
    }

    /// Étape pure de `changer_statut` avec un brouillard injecté : la clôture est refusée avec
    /// le nombre d'écritures, et rien ne change ; sans brouillard, elle passe. (La lecture réelle
    /// du brouillard, `dates_du_brouillard`, parcourt `TABLES_BROUILLARD_LUES_A_LA_CLOTURE`,
    /// vide avant la PR 13.)
    #[test]
    fn appliquer_refuse_la_cloture_avec_brouillard_injecte() {
        assert_eq!(TABLES_BROUILLARD_LUES_A_LA_CLOTURE, &[] as &[&str]);
        let mut relus = Exercices::new();
        relus.ouvrir_repris(2026).unwrap();
        let debut = jour(Month::January, 1);
        let date = jour(Month::January, 15);
        let hors_periode = jour(Month::February, 2);
        assert_eq!(
            appliquer(
                &relus,
                debut,
                Transition::Cloturer,
                vec![date, hors_periode, date]
            ),
            Err(RefusPeriode::BrouillardNonValide { nombre: 2 })
        );
        let (apres, motif) =
            appliquer(&relus, debut, Transition::Cloturer, vec![hors_periode]).unwrap();
        assert_eq!(motif, None);
        assert_ne!(apres, relus);
    }

    /// Chaque refus a son motif stable.
    #[test]
    fn chaque_refus_a_son_motif() {
        let mut relus = Exercices::new();
        relus.ouvrir_repris(2026).unwrap();
        let (janvier, fevrier, decembre) = (
            jour(Month::January, 1),
            jour(Month::February, 1),
            jour(Month::December, 1),
        );
        let motif = TexteCourt::parse("Erreur").unwrap();
        let cas = |r: &Exercices, d, t| appliquer(r, d, t, Vec::new()).map(|_| ());
        assert_eq!(
            cas(&relus, fevrier, Transition::Cloturer),
            Err(RefusPeriode::PeriodeAnterieureNonCloturee)
        );
        assert_eq!(
            cas(&relus, decembre, Transition::Cloturer),
            Err(RefusPeriode::DernierePeriode)
        );
        assert_eq!(
            cas(&relus, janvier, Transition::Deverrouiller(motif.clone())),
            Err(RefusPeriode::PeriodeNonVerrouillee)
        );
        let (verrouille, _) =
            appliquer(&relus, janvier, Transition::Verrouiller, Vec::new()).unwrap();
        assert_eq!(
            cas(&verrouille, janvier, Transition::Verrouiller),
            Err(RefusPeriode::PeriodeNonOuverte)
        );
        let (cloture, _) = appliquer(&relus, janvier, Transition::Cloturer, Vec::new()).unwrap();
        for t in [
            Transition::Verrouiller,
            Transition::Deverrouiller(motif),
            Transition::Cloturer,
        ] {
            assert_eq!(
                cas(&cloture, janvier, t),
                Err(RefusPeriode::PeriodeDejaCloturee)
            );
        }
    }
}
