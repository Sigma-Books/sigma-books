//! Brouillard persistant d'un dossier (L2-25, PR 13a ; migrations `0202_brouillard.sql` et
//! `0350_brouillard_pieces.sql` ; spec §6.1, §20 ; note d'étude de la PR 13, §1, §5).
//!
//! Un brouillon est une écriture **modifiable et supprimable**, avant sa validation (PR 13b).
//! Il peut être **déséquilibré** en cours de saisie : il ne devient une `EcritureEquilibree` qu'à
//! la validation. Ce module ne fait que lire et écrire ; les règles de saisie (compte, journal,
//! tiers, étiquette) sont celles de [`crate::serveur::routes::brouillard`], qui s'appuient sur
//! `compta-core` et sur le référentiel du dossier relu ([`super::referentiel::charger`]).
//!
//! # Stockage
//! Une ligne `brouillon`, ses lignes `brouillon_ligne` (une par position), ses contributeurs et
//! ses liens de pièces. Les montants sont des `NUMERIC` : `sqlx` n'a pas de type décimal
//! (aucun `f64`, règle absolue 2), ils transitent en texte canonique (`::text` / `::text::numeric`,
//! sans zéro final) : deux valeurs égales s'écrivent toujours de la même façon, ce qui rend la
//! comparaison d'une ligne relue et d'une ligne saisie exacte.
//!
//! # Verrous (ADR 0011, complété par la note d'étude §3)
//! Ordre : exercice, période (`FOR SHARE`, [`verrouiller_date`]), brouillon (`FOR UPDATE`,
//! [`verrouiller`]), pièces (`FOR SHARE`), puis les têtes d'audit prises par le pipeline. La
//! clôture d'une période prend la période `FOR UPDATE` puis lit les brouillons datés dedans
//! ([`dates_dans`]) : une création concurrente, qui tient la période `FOR SHARE`, est attendue
//! (et vue) ou arrive après la clôture (et est refusée).
//!
//! Niveau « requête » de l'isolation (ADR 0011) : chaque requête filtre par le cabinet et le
//! dossier du contexte, en plus de la RLS.

use std::collections::BTreeSet;

use compta_core::{Ligne, Sens};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{ErreurBase, IdCabinet, IdDossier, TransactionTenant};

/// Nombre maximal de lignes d'un brouillon (migration `0202`).
pub const LIGNES_MAX: usize = 200;

/// Nombre maximal de pièces liées à un brouillon (audit d'une suppression : au plus 200
/// identifiants cités).
pub const PIECES_MAX: usize = 200;

/// Rôle d'une pièce liée à un brouillon (spec §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolePiece {
    /// La pièce qui justifie l'écriture : au plus une par brouillon.
    Principale,
    /// Une pièce complémentaire.
    Complementaire,
}

impl RolePiece {
    /// Valeur stable de la colonne et du corps des requêtes.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Principale => "principale",
            Self::Complementaire => "complementaire",
        }
    }

    /// Lit la valeur stable.
    #[must_use]
    pub fn depuis_nom(nom: &str) -> Option<Self> {
        match nom {
            "principale" => Some(Self::Principale),
            "complementaire" => Some(Self::Complementaire),
            _ => None,
        }
    }
}

/// Motif stable du refus d'une saisie **invalide en soi** (réponse 422 de l'API, un code par
/// motif). Les contrôles qui dépendent de l'ensemble de l'écriture (équilibre, justification,
/// cohérence étiquettes et comptes de TVA) relèvent de la validation (PR 13b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusSaisie {
    /// Journal inconnu du dossier.
    JournalInconnu,
    /// Journal qui n'accepte pas la saisie (à-nouveaux : généré, jamais saisi).
    JournalSansSaisie,
    /// Date comptable hors de tout exercice du dossier.
    DateHorsExercice,
    /// Date de valeur comptable qui n'est pas antérieure à la date comptable.
    DateValeurNonAnterieure,
    /// Plus de [`LIGNES_MAX`] lignes.
    TropDeLignes,
    /// Compte inconnu du plan du dossier.
    CompteInconnu,
    /// Compte désactivé.
    CompteInactif,
    /// Compte de regroupement (il a des comptes enfants).
    CompteDeRegroupement,
    /// Montant, cours ou devise invalide (non strictement positif, devise de tenue en devise
    /// étrangère…).
    MontantInvalide,
    /// Montant de ligne qui a plus de décimales que l'échelle de la devise de tenue du dossier
    /// (ADR 0027).
    MontantEchelleTenue,
    /// Tiers inconnu du dossier.
    TiersInconnu,
    /// Tiers désactivé.
    TiersInactif,
    /// Compte hors de la fiche du tiers (un fournisseur sur un compte de la fiche 40, un
    /// client sur la fiche 41…).
    CompteHorsFicheDuTiers,
    /// Étiquette fiscale : code de forme invalide.
    EtiquetteCodeInvalide,
    /// Étiquette fiscale : taux de forme invalide.
    EtiquetteTauxInvalide,
    /// Étiquette fiscale : régime absent de la liste fermée.
    EtiquetteRegimeInconnu,
    /// Étiquette fiscale : motif manquant pour un régime exonéré.
    EtiquetteMotifManquant,
    /// Étiquette fiscale : motif interdit (régime non exonéré).
    EtiquetteMotifInterdit,
    /// Étiquette fiscale : motif étranger au régime.
    EtiquetteMotifHorsRegime,
    /// Étiquette fiscale : motif porté par une ligne de taxe.
    EtiquetteTaxeAvecMotif,
    /// Étiquette fiscale : ligne de taxe interdite pour ce régime.
    EtiquetteLigneTaxeInterdite,
    /// Étiquette fiscale : taux manquant.
    EtiquetteTauxManquant,
    /// Étiquette fiscale : taux interdit.
    EtiquetteTauxInterdit,
    /// Étiquette fiscale : taux hors de ceux du régime.
    EtiquetteTauxNonAdmis,
    /// Étiquette fiscale : exigibilité manquante.
    EtiquetteExigibiliteManquante,
    /// Étiquette fiscale : exigibilité non admise.
    EtiquetteExigibiliteNonAdmise,
}

impl RefusSaisie {
    /// Code stable du corps de la réponse.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::JournalInconnu => "journal_inconnu",
            Self::JournalSansSaisie => "journal_sans_saisie",
            Self::DateHorsExercice => "date_hors_exercice",
            Self::DateValeurNonAnterieure => "date_valeur_non_anterieure",
            Self::TropDeLignes => "trop_de_lignes",
            Self::CompteInconnu => "compte_inconnu",
            Self::CompteInactif => "compte_inactif",
            Self::CompteDeRegroupement => "compte_de_regroupement",
            Self::MontantInvalide => "montant_invalide",
            Self::MontantEchelleTenue => "montant_echelle_devise_tenue",
            Self::TiersInconnu => "tiers_inconnu",
            Self::TiersInactif => "tiers_inactif",
            Self::CompteHorsFicheDuTiers => "compte_hors_fiche_du_tiers",
            Self::EtiquetteCodeInvalide => "etiquette_code_invalide",
            Self::EtiquetteTauxInvalide => "etiquette_taux_invalide",
            Self::EtiquetteRegimeInconnu => "etiquette_regime_inconnu",
            Self::EtiquetteMotifManquant => "etiquette_motif_manquant",
            Self::EtiquetteMotifInterdit => "etiquette_motif_interdit",
            Self::EtiquetteMotifHorsRegime => "etiquette_motif_hors_regime",
            Self::EtiquetteTaxeAvecMotif => "etiquette_taxe_avec_motif",
            Self::EtiquetteLigneTaxeInterdite => "etiquette_ligne_taxe_interdite",
            Self::EtiquetteTauxManquant => "etiquette_taux_manquant",
            Self::EtiquetteTauxInterdit => "etiquette_taux_interdit",
            Self::EtiquetteTauxNonAdmis => "etiquette_taux_non_admis",
            Self::EtiquetteExigibiliteManquante => "etiquette_exigibilite_manquante",
            Self::EtiquetteExigibiliteNonAdmise => "etiquette_exigibilite_non_admise",
        }
    }

    /// Tous les motifs (test de l'unicité des codes).
    pub const TOUS: [Self; 26] = [
        Self::JournalInconnu,
        Self::JournalSansSaisie,
        Self::DateHorsExercice,
        Self::DateValeurNonAnterieure,
        Self::TropDeLignes,
        Self::CompteInconnu,
        Self::CompteInactif,
        Self::CompteDeRegroupement,
        Self::MontantInvalide,
        Self::MontantEchelleTenue,
        Self::TiersInconnu,
        Self::TiersInactif,
        Self::CompteHorsFicheDuTiers,
        Self::EtiquetteCodeInvalide,
        Self::EtiquetteTauxInvalide,
        Self::EtiquetteRegimeInconnu,
        Self::EtiquetteMotifManquant,
        Self::EtiquetteMotifInterdit,
        Self::EtiquetteMotifHorsRegime,
        Self::EtiquetteTaxeAvecMotif,
        Self::EtiquetteLigneTaxeInterdite,
        Self::EtiquetteTauxManquant,
        Self::EtiquetteTauxInterdit,
        Self::EtiquetteTauxNonAdmis,
        Self::EtiquetteExigibiliteManquante,
        Self::EtiquetteExigibiliteNonAdmise,
    ];
}

// --- Représentation stockée --------------------------------------------------------------

/// Texte décimal canonique : sans zéro final inutile (`100.50` → `100.5`, `100.00` → `100`).
#[must_use]
pub fn decimal_canonique(texte: &str) -> String {
    match texte.split_once('.') {
        Some((entier, fraction)) => {
            let fraction = fraction.trim_end_matches('0');
            if fraction.is_empty() {
                entier.to_owned()
            } else {
                format!("{entier}.{fraction}")
            }
        }
        None => texte.to_owned(),
    }
}

/// Montant en devise d'une ligne (le montant de la ligne est alors la contre-valeur XOF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviseStockee {
    /// Code ISO 4217.
    pub code: String,
    /// Montant dans la devise.
    pub montant: String,
    /// Cours appliqué.
    pub cours: String,
}

/// Étiquette fiscale d'une ligne, en valeurs stables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtiquetteStockee {
    /// `base` ou `taxe`.
    pub nature: String,
    /// `collectee` ou `deductible`.
    pub sens: String,
    /// Code du régime.
    pub regime: String,
    /// Taux en pourcent, décimal exact.
    pub taux: Option<String>,
    /// `debit` ou `encaissement`.
    pub exigibilite: Option<String>,
    /// Code du motif d'exonération.
    pub motif: Option<String>,
}

/// Ligne d'un brouillon, telle que stockée (valeurs textuelles canoniques).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneStockee {
    /// Numéro du compte.
    pub compte: String,
    /// `debit` ou `credit`.
    pub sens: &'static str,
    /// Montant en XOF.
    pub montant: String,
    /// Montant en devise étrangère.
    pub devise: Option<DeviseStockee>,
    /// Tiers (UUID).
    pub tiers: Option<Uuid>,
    /// Étiquette fiscale.
    pub etiquette: Option<EtiquetteStockee>,
}

/// Valeur stable d'un sens.
#[must_use]
pub const fn nom_sens(sens: Sens) -> &'static str {
    match sens {
        Sens::Debit => "debit",
        Sens::Credit => "credit",
    }
}

fn sens_lu(texte: &str) -> Result<&'static str, ErreurBase> {
    match texte {
        "debit" => Ok("debit"),
        "credit" => Ok("credit"),
        _ => Err(ErreurBase::Donnee("sens d'une ligne de brouillon")),
    }
}

impl LigneStockee {
    /// La ligne de `compta-core` sous sa forme stockée. Le tiers d'une ligne est un UUID
    /// ([`crate::serveur::routes::brouillard`] ne construit que cela).
    ///
    /// # Errors
    /// Tiers qui n'est pas un UUID.
    pub fn depuis(ligne: &Ligne) -> Result<Self, ErreurBase> {
        let tiers = ligne
            .tiers()
            .map(|t| Uuid::try_parse(t.as_str()))
            .transpose()
            .map_err(|_| ErreurBase::Donnee("tiers d'une ligne de brouillon"))?;
        Ok(Self {
            compte: ligne.compte().as_str().to_owned(),
            sens: nom_sens(ligne.sens()),
            montant: ligne.montant().valeur().normalize().to_string(),
            devise: ligne.devise().map(|d| DeviseStockee {
                code: d.montant().devise().code().to_owned(),
                montant: d.montant().valeur().normalize().to_string(),
                cours: d.cours().valeur().normalize().to_string(),
            }),
            tiers,
            etiquette: ligne.etiquette().map(|e| EtiquetteStockee {
                nature: e.nature().code().to_owned(),
                sens: e.sens().code().to_owned(),
                regime: e.regime().as_str().to_owned(),
                taux: e.taux().map(|t| t.to_string()),
                exigibilite: e.exigibilite().map(|x| x.code().to_owned()),
                motif: e.motif().map(|m| m.as_str().to_owned()),
            }),
        })
    }

    /// Forme compacte pour le journal d'audit (avant/après d'une ligne, un texte de 1 000 octets
    /// au plus). Le tiers est cité par son identifiant, jamais par son nom.
    #[must_use]
    pub fn texte_audit(&self) -> String {
        let mut parties = vec![format!("{} {} {}", self.compte, self.sens, self.montant)];
        if let Some(d) = &self.devise {
            parties.push(format!("devise={} {}@{}", d.code, d.montant, d.cours));
        }
        if let Some(t) = self.tiers {
            parties.push(format!("tiers={t}"));
        }
        if let Some(e) = &self.etiquette {
            let mut fiscal = format!("fiscal={}/{}/{}", e.nature, e.sens, e.regime);
            for (nom, valeur) in [
                ("taux", &e.taux),
                ("exigibilite", &e.exigibilite),
                ("motif", &e.motif),
            ] {
                if let Some(valeur) = valeur {
                    fiscal = format!("{fiscal}/{nom}={valeur}");
                }
            }
            parties.push(fiscal);
        }
        parties.join(" ")
    }
}

/// Contenu modifiable d'un brouillon : ce que la saisie fournit et que la modification remplace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContenuStocke {
    /// Journal (identifiant).
    pub journal_id: Uuid,
    /// Journal (code, pour l'affichage et l'audit).
    pub journal: String,
    /// Date comptable.
    pub date: Date,
    /// Date de valeur comptable (AUDCIF art. 22 4°).
    pub date_valeur_comptable: Option<Date>,
    /// Libellé.
    pub libelle: String,
    /// Lignes, dans l'ordre (position 1, 2, …).
    pub lignes: Vec<LigneStockee>,
    /// Devise de tenue du dossier au moment de la saisie (ISO 4217) : la validation refuse un
    /// brouillon d'une autre devise de tenue (ADR 0027).
    pub devise_tenue: String,
    /// Décimales de cette devise de tenue.
    pub decimales_tenue: u8,
}

/// Lien d'une pièce à un brouillon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LienPiece {
    /// Pièce.
    pub piece: Uuid,
    /// Rôle.
    pub role: RolePiece,
    /// Utilisateur qui l'a liée.
    pub lie_par: Uuid,
}

/// Un brouillon relu, avec ses lignes, ses pièces et ses contributeurs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrouillonStocke {
    /// Identifiant.
    pub id: Uuid,
    /// Contenu.
    pub contenu: ContenuStocke,
    /// Drapeau « à revoir ».
    pub a_revoir: bool,
    /// Qui a posé le drapeau (présent si et seulement s'il est posé).
    pub a_revoir_par: Option<Uuid>,
    /// Version (verrou optimiste).
    pub version: i32,
    /// Créateur.
    pub cree_par: Uuid,
    /// Instant de création.
    pub cree_le: OffsetDateTime,
    /// Dernier modificateur du contenu ou du drapeau.
    pub modifie_par: Uuid,
    /// Instant de la dernière modification.
    pub modifie_le: OffsetDateTime,
    /// Pièces liées.
    pub pieces: Vec<LienPiece>,
    /// Créateur et utilisateurs qui ont modifié le contenu.
    pub contributeurs: Vec<Uuid>,
}

/// Résumé d'un brouillon pour la liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeBrouillon {
    /// Identifiant.
    pub id: Uuid,
    /// Code du journal.
    pub journal: String,
    /// Date comptable.
    pub date: Date,
    /// Date de valeur comptable.
    pub date_valeur_comptable: Option<Date>,
    /// Libellé.
    pub libelle: String,
    /// Drapeau « à revoir ».
    pub a_revoir: bool,
    /// Version.
    pub version: i32,
    /// Nombre de lignes.
    pub lignes: i64,
    /// Total des débits, en XOF.
    pub debit: String,
    /// Total des crédits, en XOF.
    pub credit: String,
    /// Créateur.
    pub cree_par: Uuid,
    /// Instant de création.
    pub cree_le: OffsetDateTime,
    /// Dernier modificateur.
    pub modifie_par: Uuid,
    /// Instant de la dernière modification.
    pub modifie_le: OffsetDateTime,
}

/// Filtres de la liste.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltreBrouillard {
    /// Code du journal.
    pub journal: Option<String>,
    /// Période (identifiant) : les brouillons datés dans ses bornes.
    pub periode: Option<Uuid>,
    /// Curseur : (date comptable, identifiant) du dernier brouillon de la page précédente. Il
    /// porte la clé de tri, non une ligne à relire : le brouillon supprimé ou redaté ne vide
    /// pas la page suivante.
    pub apres: Option<(Date, Uuid)>,
}

fn tenant(tx: &TransactionTenant) -> Result<(IdCabinet, IdDossier), ErreurBase> {
    let contexte = tx.contexte();
    let dossier = contexte.id_dossier().ok_or(ErreurBase::Donnee(
        "brouillard hors du contexte d'un dossier",
    ))?;
    Ok((contexte.id_cabinet(), dossier))
}

// --- Période visée -----------------------------------------------------------------------

/// Situation de la période qui contient une date, relue **sous verrou**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodeVisee {
    /// Statut de l'exercice (`ouvert`, `en_cloture`, `cloture`).
    pub exercice: StatutVise,
    /// Statut de la période (`ouverte`, `verrouillee`, `cloturee`).
    pub periode: StatutVise,
}

/// Statut relu, sous forme de valeur stable de la colonne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatutVise {
    /// `ouvert` / `ouverte`.
    Ouvert,
    /// `en_cloture` / `verrouillee`.
    Intermediaire,
    /// `cloture` / `cloturee`.
    Clos,
}

/// Verrouille `FOR SHARE` l'exercice puis la période qui contiennent `date`, et relit leur
/// statut : création et changement de date d'un brouillon (doc de `cloturer_periode`). Une
/// clôture concurrente de la période (`FOR UPDATE`) attend la fin de cette transaction, puis
/// compte le brouillon ; une clôture déjà validée est vue ici. `None` : aucun exercice ne
/// contient la date.
///
/// # Errors
/// Contexte sans dossier, statut inconnu, erreur de PostgreSQL.
pub async fn verrouiller_date(
    tx: &mut TransactionTenant,
    date: Date,
) -> Result<Option<PeriodeVisee>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let exercice = sqlx::query_scalar!(
        "SELECT statut FROM exercice \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND debut <= $3 AND fin >= $3 \
         FOR SHARE",
        cabinet.uuid(),
        dossier.uuid(),
        date,
    )
    .fetch_optional(tx.connexion())
    .await?;
    let periode = sqlx::query_scalar!(
        "SELECT statut FROM periode \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND debut <= $3 AND fin >= $3 \
         FOR SHARE",
        cabinet.uuid(),
        dossier.uuid(),
        date,
    )
    .fetch_optional(tx.connexion())
    .await?;
    let (Some(exercice), Some(periode)) = (exercice, periode) else {
        return Ok(None);
    };
    let exercice = match exercice.as_str() {
        "ouvert" => StatutVise::Ouvert,
        "en_cloture" => StatutVise::Intermediaire,
        "cloture" => StatutVise::Clos,
        _ => return Err(ErreurBase::Donnee("statut d'exercice")),
    };
    let periode = match periode.as_str() {
        "ouverte" => StatutVise::Ouvert,
        "verrouillee" => StatutVise::Intermediaire,
        "cloturee" => StatutVise::Clos,
        _ => return Err(ErreurBase::Donnee("statut de période")),
    };
    Ok(Some(PeriodeVisee { exercice, periode }))
}

// --- Écriture ----------------------------------------------------------------------------

async fn inserer_lignes(
    tx: &mut TransactionTenant,
    id: Uuid,
    lignes: &[LigneStockee],
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    for (indice, l) in lignes.iter().enumerate() {
        let position = i16::try_from(indice + 1).map_err(|_| ErreurBase::Donnee("position"))?;
        sqlx::query!(
            r#"
            INSERT INTO brouillon_ligne
                (cabinet_id, dossier_id, brouillon_id, position, compte_numero, sens, montant,
                 devise, montant_devise, cours, tiers_id,
                 fiscal_nature, fiscal_sens, fiscal_regime, fiscal_taux, fiscal_exigibilite,
                 fiscal_motif)
            VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric,
                    $8, $9::text::numeric, $10::text::numeric, $11,
                    $12, $13, $14, $15::text::numeric, $16, $17)
            "#,
            cabinet.uuid(),
            dossier.uuid(),
            id,
            position,
            l.compte,
            l.sens,
            l.montant,
            l.devise.as_ref().map(|d| d.code.as_str()),
            l.devise.as_ref().map(|d| d.montant.as_str()),
            l.devise.as_ref().map(|d| d.cours.as_str()),
            l.tiers,
            l.etiquette.as_ref().map(|e| e.nature.as_str()),
            l.etiquette.as_ref().map(|e| e.sens.as_str()),
            l.etiquette.as_ref().map(|e| e.regime.as_str()),
            l.etiquette.as_ref().and_then(|e| e.taux.as_deref()),
            l.etiquette.as_ref().and_then(|e| e.exigibilite.as_deref()),
            l.etiquette.as_ref().and_then(|e| e.motif.as_deref()),
        )
        .execute(tx.connexion())
        .await?;
    }
    Ok(())
}

/// Enregistre `utilisateur` parmi les contributeurs du brouillon (sans effet s'il l'est déjà).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn ajouter_contributeur(
    tx: &mut TransactionTenant,
    brouillon: Uuid,
    utilisateur: Uuid,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    sqlx::query!(
        "INSERT INTO brouillon_contributeur \
             (cabinet_id, dossier_id, brouillon_id, utilisateur_id) \
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        cabinet.uuid(),
        dossier.uuid(),
        brouillon,
        utilisateur,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Crée un brouillon de l'utilisateur du contexte, avec ses lignes, et l'inscrit parmi ses
/// contributeurs. Sa version est 1.
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn creer(
    tx: &mut TransactionTenant,
    id: Uuid,
    contenu: &ContenuStocke,
    instant: OffsetDateTime,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        r#"
        INSERT INTO brouillon (cabinet_id, dossier_id, id, journal_id, date_comptable,
                               date_valeur_comptable, libelle, cree_par, cree_le, modifie_par,
                               modifie_le, devise_tenue, decimales_tenue)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $8, $9, $10, $11)
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
        contenu.journal_id,
        contenu.date,
        contenu.date_valeur_comptable,
        contenu.libelle,
        auteur,
        instant,
        contenu.devise_tenue,
        i16::from(contenu.decimales_tenue),
    )
    .execute(tx.connexion())
    .await?;
    inserer_lignes(tx, id, &contenu.lignes).await?;
    ajouter_contributeur(tx, id, auteur).await
}

/// Verrou d'un brouillon (`FOR UPDATE`) : sa version, ou `None` s'il est absent du dossier.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller(tx: &mut TransactionTenant, id: Uuid) -> Result<Option<i32>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT version FROM brouillon WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 \
         FOR UPDATE",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_optional(tx.connexion())
    .await?)
}

/// Remplace le contenu d'un brouillon **verrouillé** par [`verrouiller`] : en-tête, lignes,
/// version + 1, dernier modificateur ; l'utilisateur du contexte devient contributeur. Rend la
/// nouvelle version.
///
/// # Errors
/// Contexte sans dossier, brouillon absent, contrainte refusée par PostgreSQL.
pub async fn remplacer(
    tx: &mut TransactionTenant,
    id: Uuid,
    contenu: &ContenuStocke,
    instant: OffsetDateTime,
) -> Result<i32, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    let version = sqlx::query_scalar!(
        r#"
        UPDATE brouillon
        SET journal_id = $4, date_comptable = $5, date_valeur_comptable = $6, libelle = $7,
            version = version + 1, modifie_par = $8, modifie_le = $9
        WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3
        RETURNING version
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
        contenu.journal_id,
        contenu.date,
        contenu.date_valeur_comptable,
        contenu.libelle,
        auteur,
        instant,
    )
    .fetch_optional(tx.connexion())
    .await?
    .ok_or(ErreurBase::Donnee("brouillon à remplacer absent"))?;
    sqlx::query!(
        "DELETE FROM brouillon_ligne WHERE cabinet_id = $1 AND dossier_id = $2 \
         AND brouillon_id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .execute(tx.connexion())
    .await?;
    inserer_lignes(tx, id, &contenu.lignes).await?;
    ajouter_contributeur(tx, id, auteur).await?;
    Ok(version)
}

/// Supprime un brouillon **verrouillé** : ses lignes, contributeurs et liens, puis lui-même.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn supprimer(tx: &mut TransactionTenant, id: Uuid) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    sqlx::query!(
        "DELETE FROM brouillon_ligne WHERE cabinet_id = $1 AND dossier_id = $2 \
         AND brouillon_id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .execute(tx.connexion())
    .await?;
    sqlx::query!(
        "DELETE FROM brouillon_piece WHERE cabinet_id = $1 AND dossier_id = $2 \
         AND brouillon_id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .execute(tx.connexion())
    .await?;
    sqlx::query!(
        "DELETE FROM brouillon_contributeur WHERE cabinet_id = $1 AND dossier_id = $2 \
         AND brouillon_id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .execute(tx.connexion())
    .await?;
    sqlx::query!(
        "DELETE FROM brouillon WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Pose ou lève le drapeau « à revoir » d'un brouillon **verrouillé**, au nom de l'utilisateur
/// du contexte (enregistré comme poseur). Ni la version ni les
/// contributeurs ne changent : le drapeau n'est pas un contenu, et son auteur doit pouvoir
/// valider ensuite.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn poser_drapeau(
    tx: &mut TransactionTenant,
    id: Uuid,
    a_revoir: bool,
) -> Result<(), ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    // Le poseur est l'utilisateur du contexte ; levé, le drapeau n'a plus de poseur.
    let par = a_revoir.then(|| tx.contexte().id_utilisateur().uuid());
    sqlx::query!(
        "UPDATE brouillon SET a_revoir = $4, a_revoir_par = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
        a_revoir,
        par,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

// --- Pièces ------------------------------------------------------------------------------

/// Situation d'une pièce que l'on veut lier, relue sous `FOR SHARE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceALier {
    /// Pièce absente du dossier.
    Absente,
    /// Pièce écartée : elle ne se lie pas.
    Ecartee,
    /// Pièce liable.
    Liable,
}

/// Verrouille la pièce `FOR SHARE` (un écart concurrent attend, puis voit le lien) et dit si
/// elle est liable.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn verrouiller_piece(
    tx: &mut TransactionTenant,
    piece: Uuid,
) -> Result<PieceALier, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let ecartee = sqlx::query_scalar!(
        "SELECT ecartee_le IS NOT NULL AS \"ecartee!\" FROM piece \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 FOR SHARE",
        cabinet.uuid(),
        dossier.uuid(),
        piece,
    )
    .fetch_optional(tx.connexion())
    .await?;
    Ok(match ecartee {
        None => PieceALier::Absente,
        Some(true) => PieceALier::Ecartee,
        Some(false) => PieceALier::Liable,
    })
}

/// Liens de pièces d'un brouillon.
///
/// # Errors
/// Contexte sans dossier, rôle inconnu, erreur de PostgreSQL.
pub async fn liens_du_brouillon(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Vec<LienPiece>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query!(
        "SELECT piece_id, role, lie_par FROM brouillon_piece \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND brouillon_id = $3 \
         ORDER BY piece_id",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes
        .into_iter()
        .map(|l| {
            Ok(LienPiece {
                piece: l.piece_id,
                role: RolePiece::depuis_nom(&l.role)
                    .ok_or(ErreurBase::Donnee("rôle d'une pièce liée"))?,
                lie_par: l.lie_par,
            })
        })
        .collect()
}

/// Lie une pièce à un brouillon **verrouillé** ; l'appelant a vérifié l'absence de lien et la
/// seule principale. Le lien et le contributeur sont inscrits, la version est incrémentée ;
/// rend la nouvelle version.
///
/// # Errors
/// Contexte sans dossier, contrainte refusée par PostgreSQL.
pub async fn lier_piece(
    tx: &mut TransactionTenant,
    id: Uuid,
    piece: Uuid,
    role: RolePiece,
    instant: OffsetDateTime,
) -> Result<i32, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    sqlx::query!(
        "INSERT INTO brouillon_piece \
             (cabinet_id, dossier_id, brouillon_id, piece_id, role, lie_par) \
         VALUES ($1, $2, $3, $4, $5, $6)",
        cabinet.uuid(),
        dossier.uuid(),
        id,
        piece,
        role.nom(),
        auteur,
    )
    .execute(tx.connexion())
    .await?;
    toucher(tx, id, instant).await
}

/// Retire le lien d'une pièce à un brouillon **verrouillé** ; `false` si elle n'y était pas
/// liée. Sinon la version est incrémentée et l'utilisateur devient contributeur ; rend la
/// nouvelle version.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn retirer_piece(
    tx: &mut TransactionTenant,
    id: Uuid,
    piece: Uuid,
    instant: OffsetDateTime,
) -> Result<Option<i32>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let retires = sqlx::query!(
        "DELETE FROM brouillon_piece \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND brouillon_id = $3 AND piece_id = $4",
        cabinet.uuid(),
        dossier.uuid(),
        id,
        piece,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected();
    if retires == 0 {
        return Ok(None);
    }
    Ok(Some(toucher(tx, id, instant).await?))
}

/// Un changement de pièces est un changement de contenu : version + 1, dernier modificateur,
/// contributeur.
async fn toucher(
    tx: &mut TransactionTenant,
    id: Uuid,
    instant: OffsetDateTime,
) -> Result<i32, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let auteur = tx.contexte().id_utilisateur().uuid();
    let version = sqlx::query_scalar!(
        "UPDATE brouillon SET version = version + 1, modifie_par = $4, modifie_le = $5 \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3 RETURNING version",
        cabinet.uuid(),
        dossier.uuid(),
        id,
        auteur,
        instant,
    )
    .fetch_optional(tx.connexion())
    .await?
    .ok_or(ErreurBase::Donnee("brouillon à modifier absent"))?;
    ajouter_contributeur(tx, id, auteur).await?;
    Ok(version)
}

/// Nombre de brouillons **et d'écritures validées** du dossier du contexte auxquels la pièce est
/// liée (refus de l'écart : une pièce liée à une écriture validée est définitive, ADR 0015).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn brouillons_lies_a_la_piece(
    tx: &mut TransactionTenant,
    piece: Uuid,
) -> Result<i64, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT (SELECT count(*) FROM brouillon_piece \
                  WHERE cabinet_id = $1 AND dossier_id = $2 AND piece_id = $3) \
              + (SELECT count(*) FROM ecriture_validee_piece \
                  WHERE cabinet_id = $1 AND dossier_id = $2 AND piece_id = $3) \
               AS \"nombre!\"",
        cabinet.uuid(),
        dossier.uuid(),
        piece,
    )
    .fetch_one(tx.connexion())
    .await?)
}

// --- Lecture -----------------------------------------------------------------------------

/// Créateur et contributeurs d'un brouillon, pour la décision Cedar (séparation des tâches).
/// `None` si le brouillon est absent du dossier du contexte.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn contributeurs(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<(Uuid, Vec<Uuid>)>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let Some(createur) = sqlx::query_scalar!(
        "SELECT cree_par FROM brouillon WHERE cabinet_id = $1 AND dossier_id = $2 AND id = $3",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    let autres = sqlx::query_scalar!(
        "SELECT utilisateur_id FROM brouillon_contributeur \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND brouillon_id = $3 \
         ORDER BY utilisateur_id",
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_all(tx.connexion())
    .await?;
    let correcteurs: BTreeSet<Uuid> = autres.into_iter().filter(|u| *u != createur).collect();
    Ok(Some((createur, correcteurs.into_iter().collect())))
}

/// Un brouillon du dossier du contexte, avec ses lignes, pièces et contributeurs.
///
/// # Errors
/// Contexte sans dossier, ligne relue incohérente, erreur de PostgreSQL.
pub async fn lire(
    tx: &mut TransactionTenant,
    id: Uuid,
) -> Result<Option<BrouillonStocke>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let Some(b) = sqlx::query!(
        r#"
        SELECT b.journal_id, j.code AS journal, b.date_comptable, b.date_valeur_comptable,
               b.libelle, b.a_revoir, b.a_revoir_par, b.version, b.cree_par, b.cree_le,
               b.modifie_par, b.modifie_le, b.devise_tenue, b.decimales_tenue
        FROM brouillon b
        JOIN journal j ON j.cabinet_id = b.cabinet_id AND j.dossier_id = b.dossier_id
                      AND j.id = b.journal_id
        WHERE b.cabinet_id = $1 AND b.dossier_id = $2 AND b.id = $3
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_optional(tx.connexion())
    .await?
    else {
        return Ok(None);
    };
    let lignes = sqlx::query!(
        r#"
        SELECT compte_numero, sens, montant::text AS "montant!", devise,
               montant_devise::text AS montant_devise, cours::text AS cours, tiers_id,
               fiscal_nature, fiscal_sens, fiscal_regime, fiscal_taux::text AS fiscal_taux,
               fiscal_exigibilite, fiscal_motif
        FROM brouillon_ligne
        WHERE cabinet_id = $1 AND dossier_id = $2 AND brouillon_id = $3
        ORDER BY position
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        id,
    )
    .fetch_all(tx.connexion())
    .await?;
    let incoherent = ErreurBase::Donnee("ligne de brouillon incohérente");
    let mut relues = Vec::with_capacity(lignes.len());
    for l in lignes {
        let devise = match (l.devise, l.montant_devise, l.cours) {
            (None, None, None) => None,
            (Some(code), Some(montant), Some(cours)) => Some(DeviseStockee {
                code,
                montant: decimal_canonique(&montant),
                cours: decimal_canonique(&cours),
            }),
            _ => return Err(incoherent),
        };
        let etiquette = match (l.fiscal_nature, l.fiscal_sens, l.fiscal_regime) {
            (None, None, None) => None,
            (Some(nature), Some(sens), Some(regime)) => Some(EtiquetteStockee {
                nature,
                sens,
                regime,
                taux: l.fiscal_taux.map(|t| decimal_canonique(&t)),
                exigibilite: l.fiscal_exigibilite,
                motif: l.fiscal_motif,
            }),
            _ => return Err(incoherent),
        };
        relues.push(LigneStockee {
            compte: l.compte_numero,
            sens: sens_lu(&l.sens)?,
            montant: decimal_canonique(&l.montant),
            devise,
            tiers: l.tiers_id,
            etiquette,
        });
    }
    let pieces = liens_du_brouillon(tx, id).await?;
    let (createur, autres) = contributeurs(tx, id)
        .await?
        .ok_or(ErreurBase::Donnee("brouillon disparu pendant sa lecture"))?;
    let mut tous = vec![createur];
    tous.extend(autres);
    Ok(Some(BrouillonStocke {
        id,
        contenu: ContenuStocke {
            journal_id: b.journal_id,
            journal: b.journal,
            date: b.date_comptable,
            date_valeur_comptable: b.date_valeur_comptable,
            libelle: b.libelle,
            lignes: relues,
            devise_tenue: b.devise_tenue,
            decimales_tenue: u8::try_from(b.decimales_tenue)
                .map_err(|_| ErreurBase::Donnee("échelle de brouillon invalide"))?,
        },
        a_revoir: b.a_revoir,
        a_revoir_par: b.a_revoir_par,
        version: b.version,
        cree_par: b.cree_par,
        cree_le: b.cree_le,
        modifie_par: b.modifie_par,
        modifie_le: b.modifie_le,
        pieces,
        contributeurs: tous,
    }))
}

/// Brouillons du dossier du contexte, dans l'ordre (date comptable, identifiant), après le
/// curseur, au plus `limite` (l'appelant demande `limite + 1` pour savoir s'il en reste).
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn lister(
    tx: &mut TransactionTenant,
    filtre: &FiltreBrouillard,
    limite: u32,
) -> Result<Vec<ResumeBrouillon>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    let lignes = sqlx::query!(
        r#"
        SELECT b.id, j.code AS journal, b.date_comptable, b.date_valeur_comptable, b.libelle,
               b.a_revoir, b.version, b.cree_par, b.cree_le, b.modifie_par, b.modifie_le,
               (SELECT count(*) FROM brouillon_ligne l
                 WHERE l.cabinet_id = b.cabinet_id AND l.dossier_id = b.dossier_id
                   AND l.brouillon_id = b.id) AS "lignes!",
               (SELECT COALESCE(sum(l.montant), 0)::text FROM brouillon_ligne l
                 WHERE l.cabinet_id = b.cabinet_id AND l.dossier_id = b.dossier_id
                   AND l.brouillon_id = b.id AND l.sens = 'debit') AS "debit!",
               (SELECT COALESCE(sum(l.montant), 0)::text FROM brouillon_ligne l
                 WHERE l.cabinet_id = b.cabinet_id AND l.dossier_id = b.dossier_id
                   AND l.brouillon_id = b.id AND l.sens = 'credit') AS "credit!"
        FROM brouillon b
        JOIN journal j ON j.cabinet_id = b.cabinet_id AND j.dossier_id = b.dossier_id
                      AND j.id = b.journal_id
        WHERE b.cabinet_id = $1 AND b.dossier_id = $2
          AND ($3::text IS NULL OR j.code = $3)
          AND ($4::uuid IS NULL OR b.date_comptable BETWEEN
                (SELECT p.debut FROM periode p
                  WHERE p.cabinet_id = $1 AND p.dossier_id = $2 AND p.id = $4)
                AND (SELECT p.fin FROM periode p
                  WHERE p.cabinet_id = $1 AND p.dossier_id = $2 AND p.id = $4))
          AND ($5::date IS NULL OR (b.date_comptable, b.id) > ($5, $6))
        ORDER BY b.date_comptable, b.id
        LIMIT $7
        "#,
        cabinet.uuid(),
        dossier.uuid(),
        filtre.journal,
        filtre.periode,
        filtre.apres.map(|(d, _)| d),
        filtre.apres.map(|(_, i)| i),
        i64::from(limite),
    )
    .fetch_all(tx.connexion())
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| ResumeBrouillon {
            id: l.id,
            journal: l.journal,
            date: l.date_comptable,
            date_valeur_comptable: l.date_valeur_comptable,
            libelle: l.libelle,
            a_revoir: l.a_revoir,
            version: l.version,
            lignes: l.lignes,
            debit: decimal_canonique(&l.debit),
            credit: decimal_canonique(&l.credit),
            cree_par: l.cree_par,
            cree_le: l.cree_le,
            modifie_par: l.modifie_par,
            modifie_le: l.modifie_le,
        })
        .collect())
}

/// Date comptable de chaque brouillon du dossier du contexte daté dans `[du, au]` : la lecture
/// de la table `brouillon` à la clôture d'une période
/// ([`super::referentiel::dates_du_brouillard`]). À appeler **après** le verrou `FOR UPDATE` de
/// la période.
///
/// # Errors
/// Contexte sans dossier, erreur de PostgreSQL.
pub async fn dates_dans(
    tx: &mut TransactionTenant,
    du: Date,
    au: Date,
) -> Result<Vec<Date>, ErreurBase> {
    let (cabinet, dossier) = tenant(tx)?;
    Ok(sqlx::query_scalar!(
        "SELECT date_comptable FROM brouillon \
         WHERE cabinet_id = $1 AND dossier_id = $2 AND date_comptable BETWEEN $3 AND $4",
        cabinet.uuid(),
        dossier.uuid(),
        du,
        au,
    )
    .fetch_all(tx.connexion())
    .await?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn les_codes_de_refus_sont_distincts_et_stables() {
        let codes: BTreeSet<_> = RefusSaisie::TOUS.iter().map(|r| r.code()).collect();
        assert_eq!(codes.len(), RefusSaisie::TOUS.len());
        assert!(
            codes
                .iter()
                .all(|c| c.bytes().all(|o| o.is_ascii_lowercase() || o == b'_'))
        );
    }

    #[test]
    fn decimal_canonique_retire_les_zeros_finaux() {
        for (entree, attendu) in [
            ("100", "100"),
            ("100.00", "100"),
            ("100.50", "100.5"),
            ("0.050000", "0.05"),
            ("18.000000", "18"),
            ("5.500000", "5.5"),
        ] {
            assert_eq!(decimal_canonique(entree), attendu, "{entree}");
        }
    }

    #[test]
    fn les_roles_de_piece_font_l_aller_retour() {
        for role in [RolePiece::Principale, RolePiece::Complementaire] {
            assert_eq!(RolePiece::depuis_nom(role.nom()), Some(role));
        }
        assert_eq!(RolePiece::depuis_nom("autre"), None);
    }
}
