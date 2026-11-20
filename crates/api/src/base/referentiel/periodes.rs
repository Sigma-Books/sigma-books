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
//! Une ligne clôturée n'est ni rendue ni verrouillée par un `FOR SHARE` ou un `FOR UPDATE`
//! (politique restrictive d'irréversibilité, `0201`) : elle ne change plus, et l'état est relu
//! par une lecture simple après les verrous.
//!
//! # Brouillard
//! La clôture d'une période est refusée tant qu'une écriture du brouillard y est datée (#135).
//! Les dates sont lues **après** le verrou de la période ([`dates_du_brouillard`]). Aucune table
//! de brouillard n'existe avant la PR 13 : la liste vide est alors exacte. Fil-piège : toute table
//! `brouillon%` doit figurer dans [`TABLES_BROUILLARD_LUES_A_LA_CLOTURE`] (test de catalogue).

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
/// `brouillon%` de la base et la liste `[[brouillard]]` du catalogue soient le même ensemble. La
/// PR 13 ne peut donc pas créer sa table sans brancher sa lecture ici (et sans prendre la période
/// `FOR SHARE` à la création et au changement de date d'un brouillon).
pub const TABLES_BROUILLARD_LUES_A_LA_CLOTURE: &[&str] = &[];

/// Date comptable de chaque écriture du brouillard du dossier du contexte datée dans
/// `[du, au]`, une par écriture. À appeler **après** le verrou `FOR UPDATE` de la période.
///
/// Aucune table de brouillard n'existe encore ([`TABLES_BROUILLARD_LUES_A_LA_CLOTURE`] est
/// vide) : la liste vide est exacte.
///
/// # Errors
/// Contexte sans dossier.
#[allow(clippy::unused_async)] // Lira la base à la PR 13.
pub async fn dates_du_brouillard(
    tx: &mut TransactionTenant,
    du: Date,
    au: Date,
) -> Result<Vec<Date>, ErreurBase> {
    contexte(tx)?;
    debug_assert!(du <= au);
    Ok(Vec::new())
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

/// Échec d'une transition ou d'une ouverture.
#[derive(Debug, thiserror::Error)]
pub enum ErreurPeriode {
    /// Période absente du dossier du contexte (ou d'autrui), ou dossier sans référentiel.
    #[error("période ou référentiel absent")]
    Absente,
    /// Transition refusée par les règles de `compta-core`.
    #[error(transparent)]
    Refus(#[from] ErreurExercice),
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

    // Relecture, après les verrous : l'état committé le plus récent.
    let (relus, _, ids_periodes) = charger_exercices(tx, cabinet, dossier).await?;
    let mut apres = relus.clone();
    let motif = match transition {
        Transition::Verrouiller => {
            apres.verrouiller_periode(debut)?;
            None
        }
        Transition::Deverrouiller(motif) => Some(apres.deverrouiller_periode(debut, motif)?.motif),
        Transition::Cloturer => {
            let fin = relus
                .exercice_contenant(debut)
                .and_then(|e| e.periode_contenant(debut))
                .map(compta_core::Periode::fin)
                .ok_or(ErreurPeriode::Absente)?;
            let brouillard = dates_du_brouillard(tx, debut, fin).await?;
            apres.cloturer_periode(debut, brouillard)?;
            None
        }
    };

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
    let exercice = exercices.ouvrir_suivant()?.clone();
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
    use compta_core::{ErreurExercice, Exercices};
    use time::{Date, Month};

    use super::TABLES_BROUILLARD_LUES_A_LA_CLOTURE;

    /// Clôture avec brouillard, au niveau pur : `changer_statut` passe les dates lues à
    /// `cloturer_periode`, qui refuse et ne change rien. Aucune table de brouillard avant la
    /// PR 13 : la liste lue est vide (fil-piège : `tests/catalogue.rs`).
    #[test]
    fn une_cloture_avec_brouillard_est_refusee_sans_effet() {
        assert_eq!(TABLES_BROUILLARD_LUES_A_LA_CLOTURE, &[] as &[&str]);
        let mut exercices = Exercices::new();
        exercices.ouvrir_repris(2026).unwrap();
        let avant = exercices.clone();
        let debut = Date::from_calendar_date(2026, Month::January, 1).unwrap();
        let date = Date::from_calendar_date(2026, Month::January, 15).unwrap();
        assert!(matches!(
            exercices.cloturer_periode(debut, [date, date]),
            Err(ErreurExercice::BrouillardNonValide { nombre: 2, .. })
        ));
        assert_eq!(exercices, avant);
        exercices.cloturer_periode(debut, []).unwrap();
    }
}
