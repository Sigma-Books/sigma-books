//! Liens à usage unique d'invitation et de réinitialisation (ADR 0012, 0023 ; table `lien_acces`,
//! migration `0102_administration.sql`).
//!
//! - **Émission** ([`emettre`]) : dans la transaction d'un cabinet (administration, ou file de
//!   tâches de la PR 9b). Le lien (256 bits) n'est rendu qu'à l'appelant, qui le remet par
//!   courriel ; seule son **empreinte** (SHA-256 avec séparation de domaine) est stockée. Un
//!   nouveau lien ferme le précédent de la même cible.
//! - **Usage** ([`TransactionLien`]) : le porteur n'a ni session ni cabinet. La transaction ne
//!   pose que `sigma.jeton_lien` (empreinte en hexadécimal) ; elle ne voit que cette ligne. Le
//!   lien vérifié (vivant, non fermé) est consommé, puis le contexte passe au cabinet du lien :
//!   cette **preuve** de possession est ce qui autorise à poser le cabinet et l'utilisateur,
//!   exactement comme un facteur vérifié le fait à la connexion.
//!
//! Un lien invalide, expiré, fermé ou inconnu : la même issue ([`ErreurLien::Invalide`]).

use base64ct::{Base64UrlUnpadded, Encoding};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::Zeroize;

use super::{
    ErreurBase, IdAppartenance, IdCabinet, IdUtilisateur, TransactionTenant, TransactionUtilisateur,
};

const OCTETS: usize = 32;
/// Longueur du texte d'un lien (32 octets en base64url sans bourrage).
pub const LONGUEUR_TEXTE: usize = 43;
const DOMAINE: &[u8] = b"sigma-books lien v1\0";

/// Validité d'un lien d'invitation (ADR 0012).
pub const VALIDITE_INVITATION: time::Duration = time::Duration::hours(24);
/// Validité d'un lien de réinitialisation (ADR 0012).
pub const VALIDITE_REINITIALISATION: time::Duration = time::Duration::hours(1);

/// Nature d'un lien.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatureLien {
    /// Invitation à rejoindre un cabinet.
    Invitation,
    /// Réinitialisation de l'accès d'un membre.
    Reinitialisation,
}

impl NatureLien {
    /// Nom enregistré.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Invitation => "invitation",
            Self::Reinitialisation => "reinitialisation",
        }
    }

    fn depuis_nom(nom: &str) -> Option<Self> {
        [Self::Invitation, Self::Reinitialisation]
            .into_iter()
            .find(|n| n.nom() == nom)
    }

    /// Validité du lien de cette nature.
    #[must_use]
    pub const fn validite(self) -> time::Duration {
        match self {
            Self::Invitation => VALIDITE_INVITATION,
            Self::Reinitialisation => VALIDITE_REINITIALISATION,
        }
    }
}

/// Lien en clair : 32 octets secrets, rendus une seule fois à l'émetteur. Ni `Display`, ni
/// `Debug` lisible ; effacé de la mémoire à sa libération.
pub struct JetonLien([u8; OCTETS]);

impl Drop for JetonLien {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for JetonLien {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JetonLien(<masqué>)")
    }
}

impl JetonLien {
    /// Tire un lien (aléa du système).
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si le système ne fournit pas d'aléa.
    pub fn tirer() -> Result<Self, ErreurBase> {
        let mut octets = [0u8; OCTETS];
        getrandom::fill(&mut octets).map_err(|_| ErreurBase::Donnee("aléa du système"))?;
        Ok(Self(octets))
    }

    /// Lit un lien présenté : exactement 43 caractères base64url. `None` sinon.
    #[must_use]
    pub fn depuis_texte(texte: &str) -> Option<Self> {
        if texte.len() != LONGUEUR_TEXTE {
            return None;
        }
        let mut octets = [0u8; OCTETS];
        let lus = Base64UrlUnpadded::decode(texte, &mut octets).ok()?;
        (lus.len() == OCTETS).then_some(Self(octets))
    }

    /// Forme transmise (dans le courriel). À ne jamais journaliser.
    #[must_use]
    pub fn texte(&self) -> String {
        Base64UrlUnpadded::encode_string(&self.0)
    }

    /// Empreinte, seule forme stockée.
    #[must_use]
    pub fn empreinte(&self) -> EmpreinteLien {
        let mut h = Sha256::new();
        h.update(DOMAINE);
        h.update(self.0);
        EmpreinteLien(h.finalize().into())
    }
}

/// Empreinte d'un lien (jamais affichée).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EmpreinteLien([u8; 32]);

impl EmpreinteLien {
    /// Octets de l'empreinte.
    #[must_use]
    pub const fn octets(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for EmpreinteLien {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EmpreinteLien(<masquée>)")
    }
}

/// Émet un lien pour `cible` (invitation ou réinitialisation du cabinet du contexte) valable
/// `nature.validite()` ; ferme les liens encore vivants de la même cible.
///
/// # Errors
/// Aléa indisponible, erreur de PostgreSQL.
pub async fn emettre(
    tx: &mut TransactionTenant,
    nature: NatureLien,
    cible: Uuid,
    instant: OffsetDateTime,
) -> Result<JetonLien, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    fermer(tx, nature, cible, instant, "remplace").await?;
    let jeton = JetonLien::tirer()?;
    let empreinte = jeton.empreinte();
    sqlx::query!(
        "INSERT INTO lien_acces (id, nature, cabinet_cible, cible, jeton_hache, cree_le, expire_le) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        Uuid::new_v4(),
        nature.nom(),
        cabinet,
        cible,
        empreinte.octets().as_slice(),
        instant,
        instant + nature.validite(),
    )
    .execute(tx.connexion())
    .await?;
    Ok(jeton)
}

/// Ferme les liens vivants d'une cible (`remplace`, `annule` ou `utilise`).
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn fermer(
    tx: &mut TransactionTenant,
    nature: NatureLien,
    cible: Uuid,
    instant: OffsetDateTime,
    raison: &str,
) -> Result<u64, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query!(
        "UPDATE lien_acces SET ferme_le = $4, raison = $5 \
         WHERE cabinet_cible = $1 AND nature = $2 AND cible = $3 AND ferme_le IS NULL",
        cabinet,
        nature.nom(),
        cible,
        instant,
        raison,
    )
    .execute(tx.connexion())
    .await?
    .rows_affected())
}

/// Nombre de liens vivants (non fermés, non expirés) d'une cible du cabinet du contexte.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn compter_vivants(
    tx: &mut TransactionTenant,
    nature: NatureLien,
    cible: Uuid,
    instant: OffsetDateTime,
) -> Result<i64, ErreurBase> {
    let cabinet = tx.contexte().id_cabinet().uuid();
    Ok(sqlx::query_scalar!(
        "SELECT count(*) AS \"n!\" FROM lien_acces \
         WHERE cabinet_cible = $1 AND nature = $2 AND cible = $3 \
           AND ferme_le IS NULL AND expire_le > $4",
        cabinet,
        nature.nom(),
        cible,
        instant,
    )
    .fetch_one(tx.connexion())
    .await?)
}

/// Échec de l'usage d'un lien.
#[derive(Debug, thiserror::Error)]
pub enum ErreurLien {
    /// Lien inconnu, expiré, fermé, ou dont la cible n'est plus en attente : une seule issue.
    #[error("lien invalide")]
    Invalide,
    /// Un compte existe déjà pour le courriel de l'invitation : il l'accepte connecté.
    #[error("compte existant")]
    CompteExistant,
    /// Erreur d'accès à la base.
    #[error(transparent)]
    Base(#[from] ErreurBase),
}

impl From<sqlx::Error> for ErreurLien {
    fn from(e: sqlx::Error) -> Self {
        Self::Base(e.into())
    }
}

/// Un lien présenté et vérifié (vivant), pas encore consommé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LienLu {
    id: Uuid,
    /// Nature.
    pub nature: NatureLien,
    /// Cabinet du lien.
    pub cabinet: IdCabinet,
    /// Invitation ou réinitialisation visée.
    pub cible: Uuid,
}

/// Ce que l'usage d'un lien a changé, pour l'audit et la suite.
#[derive(Debug)]
pub struct Utilisation {
    /// Transaction au contexte du seul utilisateur concerné, à poursuivre (session, audit).
    pub tx: TransactionUtilisateur,
    /// Utilisateur concerné.
    pub utilisateur: IdUtilisateur,
    /// Cabinet du lien.
    pub cabinet: IdCabinet,
    /// Nature du lien.
    pub nature: NatureLien,
    /// Invitation ou réinitialisation.
    pub cible: Uuid,
    /// Rôle donné (invitation).
    pub role: Option<String>,
}

/// Transaction d'un porteur de lien : le seul `sigma.jeton_lien` est posé.
#[derive(Debug)]
pub struct TransactionLien {
    tx: Transaction<'static, Postgres>,
}

/// Ouvre la transaction d'un porteur de lien : cabinet, dossier, utilisateur et jeton de session
/// vides ; `sigma.jeton_lien` posé à l'empreinte.
///
/// # Errors
/// Connexion indisponible ou erreur de PostgreSQL.
pub(crate) async fn ouvrir_transaction_lien(
    pool: &sqlx::PgPool,
    empreinte: &EmpreinteLien,
) -> Result<TransactionLien, ErreurBase> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "SELECT set_config('sigma.cabinet', '', true) AS cabinet, \
                set_config('sigma.dossier', '', true) AS dossier, \
                set_config('sigma.jeton_session', '', true) AS jeton, \
                set_config('sigma.utilisateur', '', true) AS utilisateur, \
                set_config('sigma.jeton_lien', encode($1::bytea, 'hex'), true) AS lien",
        empreinte.octets().as_slice(),
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(TransactionLien { tx })
}

impl TransactionLien {
    /// Lit le lien présenté s'il est vivant à `instant`.
    ///
    /// # Errors
    /// Erreur de PostgreSQL.
    pub async fn lire(&mut self, instant: OffsetDateTime) -> Result<Option<LienLu>, ErreurBase> {
        let l = sqlx::query!(
            "SELECT id, nature, cabinet_cible, cible FROM lien_acces \
             WHERE ferme_le IS NULL AND expire_le > $1 FOR UPDATE",
            instant,
        )
        .fetch_optional(&mut *self.tx)
        .await?;
        l.map(|l| {
            Ok(LienLu {
                id: l.id,
                nature: NatureLien::depuis_nom(&l.nature)
                    .ok_or(ErreurBase::Donnee("nature de lien inconnue"))?,
                cabinet: IdCabinet::depuis_uuid(l.cabinet_cible),
                cible: l.cible,
            })
        })
        .transpose()
    }

    /// Termine sans rien changer.
    ///
    /// # Errors
    /// Erreur de PostgreSQL à la validation.
    pub async fn confirmer(self) -> Result<(), ErreurBase> {
        self.tx.commit().await?;
        Ok(())
    }

    async fn consommer(
        &mut self,
        lien: &LienLu,
        instant: OffsetDateTime,
    ) -> Result<(), ErreurLien> {
        let n = sqlx::query!(
            "UPDATE lien_acces SET ferme_le = $2, raison = 'utilise' \
             WHERE id = $1 AND ferme_le IS NULL AND expire_le > $2",
            lien.id,
            instant,
        )
        .execute(&mut *self.tx)
        .await?
        .rows_affected();
        if n != 1 {
            return Err(ErreurLien::Invalide);
        }
        // Le lien est une preuve de possession : le cabinet du lien devient le contexte.
        sqlx::query!(
            "SELECT set_config('sigma.cabinet', $1, true) AS cabinet",
            lien.cabinet.to_string(),
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(())
    }

    async fn poser_utilisateur(&mut self, utilisateur: IdUtilisateur) -> Result<(), ErreurLien> {
        sqlx::query!(
            "SELECT set_config('sigma.utilisateur', $1, true) AS utilisateur",
            utilisateur.to_string(),
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(())
    }

    /// Vrai si l'utilisateur du contexte a une appartenance à un autre cabinet que `cabinet` : la
    /// lecture se fait sans cabinet en contexte (politique `appartenance_membre`), puis le
    /// cabinet est rétabli.
    async fn membre_d_un_autre_cabinet(
        &mut self,
        cabinet: IdCabinet,
        utilisateur: IdUtilisateur,
    ) -> Result<bool, ErreurLien> {
        sqlx::query!("SELECT set_config('sigma.cabinet', '', true) AS cabinet")
            .fetch_one(&mut *self.tx)
            .await?;
        let n = sqlx::query_scalar!(
            "SELECT count(*) AS \"n!\" FROM appartenance WHERE utilisateur_id = $1 AND cabinet_id <> $2",
            utilisateur.uuid(),
            cabinet.uuid(),
        )
        .fetch_one(&mut *self.tx)
        .await?;
        sqlx::query!(
            "SELECT set_config('sigma.cabinet', $1, true) AS cabinet",
            cabinet.to_string()
        )
        .fetch_one(&mut *self.tx)
        .await?;
        Ok(n > 0)
    }

    async fn sortir_du_cabinet(mut self) -> Result<Transaction<'static, Postgres>, ErreurLien> {
        sqlx::query!("SELECT set_config('sigma.cabinet', '', true) AS cabinet")
            .fetch_one(&mut *self.tx)
            .await?;
        Ok(self.tx)
    }

    /// Consomme un lien présenté **hors session** : invitation d'un nouveau compte (créé ici, à
    /// l'identifiant `nouveau`) ou réinitialisation d'un membre (ses passkeys, son secours, ses
    /// codes et ses sessions sont révoqués). Tout se fait dans cette transaction, validée par
    /// l'appelant après la session et l'audit.
    ///
    /// # Errors
    /// [`ErreurLien::Invalide`], [`ErreurLien::CompteExistant`], erreur de base.
    #[allow(clippy::too_many_lines)] // Deux natures de lien, une transaction : lecture linéaire.
    pub async fn utiliser(
        mut self,
        lien: LienLu,
        instant: OffsetDateTime,
        nouveau: IdUtilisateur,
    ) -> Result<Utilisation, ErreurLien> {
        self.consommer(&lien, instant).await?;
        let (utilisateur, role) = match lien.nature {
            NatureLien::Invitation => {
                let i = sqlx::query!(
                    "SELECT courriel, nom, role FROM invitation \
                     WHERE cabinet_id = $1 AND id = $2 AND etat = 'en_attente' FOR UPDATE",
                    lien.cabinet.uuid(),
                    lien.cible,
                )
                .fetch_optional(&mut *self.tx)
                .await?
                .ok_or(ErreurLien::Invalide)?;
                // Courriel déjà pris (compte d'un autre cabinet, invisible sous RLS) : l'index
                // unique le dit ; la transaction est abandonnée, le lien reste vivant.
                let cree = sqlx::query!(
                    "INSERT INTO utilisateur (id, courriel, nom) VALUES ($1, $2, $3)",
                    nouveau.uuid(),
                    i.courriel,
                    i.nom,
                )
                .execute(&mut *self.tx)
                .await;
                match cree {
                    Ok(_) => {}
                    Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                        return Err(ErreurLien::CompteExistant);
                    }
                    Err(e) => return Err(e.into()),
                }
                self.poser_utilisateur(nouveau).await?;
                sqlx::query!(
                    "INSERT INTO appartenance (cabinet_id, utilisateur_id, role, id) \
                     VALUES ($1, $2, $3, $4)",
                    lien.cabinet.uuid(),
                    nouveau.uuid(),
                    i.role,
                    IdAppartenance::nouveau().uuid(),
                )
                .execute(&mut *self.tx)
                .await?;
                sqlx::query!(
                    "UPDATE invitation SET etat = 'acceptee', traitee_le = $3, utilisateur_id = $4 \
                     WHERE cabinet_id = $1 AND id = $2",
                    lien.cabinet.uuid(),
                    lien.cible,
                    instant,
                    nouveau.uuid(),
                )
                .execute(&mut *self.tx)
                .await?;
                alerter_dans(&mut self.tx, lien.cabinet, "droits_modifies", nouveau).await?;
                (nouveau, Some(i.role))
            }
            NatureLien::Reinitialisation => {
                let r = sqlx::query!(
                    "SELECT r.utilisateur_id, a.actif FROM reinitialisation r \
                     JOIN appartenance a \
                       ON a.cabinet_id = r.cabinet_id AND a.utilisateur_id = r.utilisateur_id \
                     WHERE r.cabinet_id = $1 AND r.id = $2 AND r.etat = 'en_attente' \
                       AND r.echeance <= $3 FOR UPDATE OF r",
                    lien.cabinet.uuid(),
                    lien.cible,
                    instant,
                )
                .fetch_optional(&mut *self.tx)
                .await?
                .ok_or(ErreurLien::Invalide)?;
                if !r.actif {
                    return Err(ErreurLien::Invalide);
                }
                let utilisateur = IdUtilisateur::depuis_uuid(r.utilisateur_id);
                sqlx::query!(
                    "UPDATE reinitialisation SET etat = 'aboutie', traitee_le = $3 \
                     WHERE cabinet_id = $1 AND id = $2",
                    lien.cabinet.uuid(),
                    lien.cible,
                    instant,
                )
                .execute(&mut *self.tx)
                .await?;
                self.poser_utilisateur(utilisateur).await?;
                // Le compte a pu rejoindre un autre cabinet pendant le délai : ses facteurs ne
                // se retirent pas pour un cabinet qui n'est qu'un des siens (ADR 0023, §4).
                if self
                    .membre_d_un_autre_cabinet(lien.cabinet, utilisateur)
                    .await?
                {
                    return Err(ErreurLien::Invalide);
                }
                retirer_les_facteurs(&mut self.tx, utilisateur, instant).await?;
                alerter_dans(
                    &mut self.tx,
                    lien.cabinet,
                    "reinitialisation_aboutie",
                    utilisateur,
                )
                .await?;
                (utilisateur, None)
            }
        };
        let cabinet = lien.cabinet;
        let tx = self.sortir_du_cabinet().await?;
        Ok(Utilisation {
            tx: TransactionUtilisateur { tx, utilisateur },
            utilisateur,
            cabinet,
            nature: lien.nature,
            cible: lien.cible,
            role,
        })
    }

    /// Consomme une invitation présentée par un utilisateur **déjà authentifié** (session
    /// complète) : son compte garde son identité et gagne l'appartenance. Le courriel de
    /// l'invitation doit être celui de son compte, sinon [`ErreurLien::Invalide`] (même issue
    /// qu'un lien inconnu : rien ne dit qu'une invitation existe pour un autre).
    ///
    /// `utilisateur` doit être celui de la session authentifiée par le pipeline.
    ///
    /// # Errors
    /// [`ErreurLien::Invalide`], erreur de base.
    pub(crate) async fn utiliser_connecte(
        mut self,
        lien: LienLu,
        instant: OffsetDateTime,
        utilisateur: IdUtilisateur,
    ) -> Result<Utilisation, ErreurLien> {
        if lien.nature != NatureLien::Invitation {
            return Err(ErreurLien::Invalide);
        }
        self.consommer(&lien, instant).await?;
        self.poser_utilisateur(utilisateur).await?;
        let moi = sqlx::query!(
            "SELECT courriel FROM utilisateur WHERE id = $1",
            utilisateur.uuid()
        )
        .fetch_optional(&mut *self.tx)
        .await?
        .ok_or(ErreurLien::Invalide)?;
        let i = sqlx::query!(
            "SELECT courriel, role FROM invitation \
             WHERE cabinet_id = $1 AND id = $2 AND etat = 'en_attente' FOR UPDATE",
            lien.cabinet.uuid(),
            lien.cible,
        )
        .fetch_optional(&mut *self.tx)
        .await?
        .ok_or(ErreurLien::Invalide)?;
        if i.courriel.to_lowercase() != moi.courriel.to_lowercase() {
            return Err(ErreurLien::Invalide);
        }
        let n = sqlx::query!(
            "INSERT INTO appartenance (cabinet_id, utilisateur_id, role, id) \
             VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            lien.cabinet.uuid(),
            utilisateur.uuid(),
            i.role,
            IdAppartenance::nouveau().uuid(),
        )
        .execute(&mut *self.tx)
        .await?
        .rows_affected();
        if n != 1 {
            return Err(ErreurLien::Invalide);
        }
        sqlx::query!(
            "UPDATE invitation SET etat = 'acceptee', traitee_le = $3, utilisateur_id = $4 \
             WHERE cabinet_id = $1 AND id = $2",
            lien.cabinet.uuid(),
            lien.cible,
            instant,
            utilisateur.uuid(),
        )
        .execute(&mut *self.tx)
        .await?;
        alerter_dans(&mut self.tx, lien.cabinet, "droits_modifies", utilisateur).await?;
        let cabinet = lien.cabinet;
        let tx = self.sortir_du_cabinet().await?;
        Ok(Utilisation {
            tx: TransactionUtilisateur { tx, utilisateur },
            utilisateur,
            cabinet,
            nature: lien.nature,
            cible: lien.cible,
            role: Some(i.role),
        })
    }
}

/// Crée une alerte (`notification`) du cabinet du lien, dans la transaction de l'usage : le
/// cabinet est déjà posé (politique d'insertion du tenant). `sujet` : un sujet sans dossier.
async fn alerter_dans(
    tx: &mut Transaction<'static, Postgres>,
    cabinet: IdCabinet,
    sujet: &'static str,
    concerne: IdUtilisateur,
) -> Result<(), ErreurLien> {
    sqlx::query!(
        "INSERT INTO tache (id, nature, sujet, cabinet_cible, objet, etat, cree_le, \
                            prochaine_tentative) \
         VALUES ($1, 'notification', $2, $3, $4, 'en_attente', now(), now())",
        Uuid::new_v4(),
        sujet,
        cabinet.uuid(),
        concerne.uuid(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Retire les passkeys, le secours et les codes de récupération de l'utilisateur du contexte, et
/// révoque ses sessions (réinitialisation, ADR 0012). Aucun facteur n'est conservé : le lien
/// n'enrôle qu'une passkey neuve.
async fn retirer_les_facteurs(
    tx: &mut Transaction<'static, Postgres>,
    utilisateur: IdUtilisateur,
    instant: OffsetDateTime,
) -> Result<(), ErreurLien> {
    let u = utilisateur.uuid();
    sqlx::query!("DELETE FROM facteur_passkey WHERE utilisateur_id = $1", u)
        .execute(&mut **tx)
        .await?;
    sqlx::query!("DELETE FROM defi_webauthn WHERE utilisateur_id = $1", u)
        .execute(&mut **tx)
        .await?;
    sqlx::query!("DELETE FROM facteur_secours WHERE utilisateur_id = $1", u)
        .execute(&mut **tx)
        .await?;
    sqlx::query!("DELETE FROM code_recuperation WHERE utilisateur_id = $1", u)
        .execute(&mut **tx)
        .await?;
    sqlx::query!(
        "UPDATE session SET revoquee_le = $2, motif_revocation = 'reinitialisation' \
         WHERE utilisateur_id = $1 AND revoquee_le IS NULL",
        u,
        instant,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_texte_du_lien_a_43_caracteres_et_se_relit() {
        let j = JetonLien::tirer().unwrap();
        let texte = j.texte();
        assert_eq!(texte.len(), LONGUEUR_TEXTE);
        let relu = JetonLien::depuis_texte(&texte).unwrap();
        assert_eq!(relu.empreinte(), j.empreinte());
    }

    #[test]
    fn deux_liens_different_et_les_textes_invalides_sont_refuses() {
        let (a, b) = (JetonLien::tirer().unwrap(), JetonLien::tirer().unwrap());
        assert_ne!(a.empreinte(), b.empreinte());
        for faux in [
            "",
            "court",
            &"é".repeat(43),
            &"!".repeat(43),
            &"A".repeat(44),
        ] {
            assert!(JetonLien::depuis_texte(faux).is_none(), "{faux}");
        }
    }

    #[test]
    fn l_empreinte_est_separee_de_celle_d_une_session() {
        let j = JetonLien::tirer().unwrap();
        let nu: [u8; 32] = Sha256::digest(j.0).into();
        assert_ne!(j.empreinte().octets(), &nu);
    }

    #[test]
    fn le_debug_ne_montre_rien() {
        let j = JetonLien::tirer().unwrap();
        assert_eq!(format!("{j:?}"), "JetonLien(<masqué>)");
        assert!(!format!("{:?}", j.empreinte()).contains('['));
    }
}
