//! Sessions (ADR 0012 ; L2-17) : table d'instance `session`, sous RLS (migration
//! `0004_sessions.sql`).
//!
//! Deux accès seulement :
//! - **par l'empreinte du jeton présenté** ([`lire_par_empreinte`]) : une
//!   courte transaction qui ne pose que `sigma.jeton_session` (empreinte en hexadécimal) et ne
//!   voit que cette session. C'est l'authentification d'une requête ;
//! - **au contexte de l'utilisateur** ([`TransactionUtilisateur`]) : création et purge, liste,
//!   rotation, révocation de ses propres sessions. Chaque requête filtre aussi par l'utilisateur
//!   du contexte : celles d'un autre sont absentes.
//!
//! Le jeton lui-même n'arrive jamais ici : seulement son empreinte SHA-256 ([`EmpreinteJeton`]).
//! Les instants sont fournis par l'appelant (horloge injectable).

use std::fmt;

use time::{Duration, OffsetDateTime};

use super::{ErreurBase, IdSession, IdUtilisateur, TransactionUtilisateur};

/// Nature d'une session (ADR 0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatureSession {
    /// Ouverte par un facteur fort : accès aux routes métier.
    Complete,
    /// Ouverte par un code de récupération : enrôlement d'une passkey seulement (F1.12).
    Restreinte,
    /// Ouverte par un lien d'invitation ou de réinitialisation : enrôlement seulement.
    Enrolement,
}

impl NatureSession {
    /// Nom enregistré en base et rendu à l'interface.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Restreinte => "restreinte",
            Self::Enrolement => "enrolement",
        }
    }

    fn depuis_nom(nom: &str) -> Option<Self> {
        [Self::Complete, Self::Restreinte, Self::Enrolement]
            .into_iter()
            .find(|n| n.nom() == nom)
    }
}

/// Facteur vérifié à l'ouverture d'une session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodeSession {
    /// Passkey (WebAuthn), méthode principale (PR 8c).
    Passkey,
    /// Secours : mot de passe et TOTP (PR 8b).
    Secours,
    /// Code de récupération (PR 8b) : session restreinte.
    CodeRecuperation,
    /// Lien d'invitation ou de réinitialisation (PR 9) : session d'enrôlement.
    LienEnrolement,
}

impl MethodeSession {
    /// Nom enregistré en base.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Passkey => "passkey",
            Self::Secours => "secours",
            Self::CodeRecuperation => "code_recuperation",
            Self::LienEnrolement => "lien_enrolement",
        }
    }

    fn depuis_nom(nom: &str) -> Option<Self> {
        [
            Self::Passkey,
            Self::Secours,
            Self::CodeRecuperation,
            Self::LienEnrolement,
        ]
        .into_iter()
        .find(|m| m.nom() == nom)
    }
}

/// Motif d'une révocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotifRevocation {
    /// Déconnexion de la session courante.
    Deconnexion,
    /// Révocation par l'utilisateur, depuis la liste de ses sessions.
    RevocationUtilisateur,
    /// Changement de facteur de l'utilisateur (enrôlement du secours, nouvelle série de
    /// codes) : ses autres sessions sont révoquées (ADR 0012).
    ChangementFacteur,
    /// Révocation par un administrateur du cabinet, depuis la liste des sessions d'un membre.
    RevocationAdministrateur,
    /// Changement de droits ou suspension du membre : ses sessions sont révoquées (ADR 0012).
    ChangementDroits,
    /// Réinitialisation de l'accès par un administrateur (ADR 0012).
    Reinitialisation,
}

impl MotifRevocation {
    /// Nom enregistré en base.
    #[must_use]
    pub const fn nom(self) -> &'static str {
        match self {
            Self::Deconnexion => "deconnexion",
            Self::RevocationUtilisateur => "revocation_utilisateur",
            Self::ChangementFacteur => "changement_facteur",
            Self::RevocationAdministrateur => "revocation_administrateur",
            Self::ChangementDroits => "changement_droits",
            Self::Reinitialisation => "reinitialisation",
        }
    }
}

/// Durées des sessions (ADR 0012). [`Default`] donne les valeurs de l'ADR ; la configuration ne
/// peut que les raccourcir ([`DureesSession::raccourcies`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DureesSession {
    /// Inactivité maximale (30 minutes).
    pub inactivite: Duration,
    /// Durée maximale d'une session complète depuis son ouverture (12 heures).
    pub duree_max: Duration,
    /// Durée maximale d'une session restreinte ou d'enrôlement (15 minutes, choix technique :
    /// elle ne sert qu'à enrôler une passkey).
    pub duree_max_enrolement: Duration,
    /// Intervalle minimal entre deux écritures de `derniere_activite` (60 secondes) : une
    /// requête sur deux ne réécrit pas la ligne.
    pub pas_activite: Duration,
    /// Conservation d'une session close (expirée ou révoquée) avant sa purge : 30 jours par
    /// défaut, **à confirmer par le mainteneur** (adresse IP conservée, loi 2008-12).
    pub conservation: Duration,
}

impl Default for DureesSession {
    fn default() -> Self {
        Self {
            inactivite: Duration::minutes(30),
            duree_max: Duration::hours(12),
            duree_max_enrolement: Duration::minutes(15),
            pas_activite: Duration::seconds(60),
            conservation: Duration::days(30),
        }
    }
}

impl DureesSession {
    /// Ces durées, chacune bornée par celle de l'ADR 0012 : la configuration ne peut que les
    /// raccourcir. La conservation n'est pas bornée (elle relève de la loi, non de la sécurité).
    #[must_use]
    pub fn raccourcies(self) -> Self {
        let adr = Self::default();
        Self {
            inactivite: self.inactivite.min(adr.inactivite),
            duree_max: self.duree_max.min(adr.duree_max),
            duree_max_enrolement: self.duree_max_enrolement.min(adr.duree_max_enrolement),
            pas_activite: self.pas_activite.min(adr.pas_activite),
            conservation: self.conservation,
        }
    }

    /// Durée maximale d'une session de cette nature.
    #[must_use]
    pub fn duree_max_de(&self, nature: NatureSession) -> Duration {
        match nature {
            NatureSession::Complete => self.duree_max,
            NatureSession::Restreinte | NatureSession::Enrolement => self.duree_max_enrolement,
        }
    }
}

/// Empreinte SHA-256 d'un jeton de session : seule forme du jeton stockée en base. Jamais
/// affichée (`Debug` masqué).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EmpreinteJeton([u8; 32]);

impl EmpreinteJeton {
    /// Reprend une empreinte calculée.
    #[must_use]
    pub const fn new(octets: [u8; 32]) -> Self {
        Self(octets)
    }

    fn octets(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for EmpreinteJeton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EmpreinteJeton(<masquée>)")
    }
}

/// Une session lue en base (sans son empreinte).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Identifiant public.
    pub id: IdSession,
    /// Titulaire.
    pub utilisateur: IdUtilisateur,
    /// Nature.
    pub nature: NatureSession,
    /// Facteur vérifié à l'ouverture.
    pub methode: MethodeSession,
    /// Ouverture.
    pub cree_le: OffsetDateTime,
    /// Dernière activité enregistrée (à [`DureesSession::pas_activite`] près).
    pub derniere_activite: OffsetDateTime,
    /// Dernière vérification de facteur (ouverture ou ré-authentification).
    pub derniere_verification: OffsetDateTime,
    /// Révocation, si elle a eu lieu.
    pub revoquee_le: Option<OffsetDateTime>,
    /// Appareil déclaré à l'ouverture.
    pub appareil: Option<String>,
    /// Adresse du client à l'ouverture.
    pub adresse: Option<String>,
}

impl Session {
    /// Fin absolue : ouverture plus la durée maximale de sa nature.
    #[must_use]
    pub fn fin_absolue(&self, durees: &DureesSession) -> OffsetDateTime {
        self.cree_le + durees.duree_max_de(self.nature)
    }

    /// Fin par inactivité : dernière activité plus l'inactivité maximale.
    #[must_use]
    pub fn fin_inactivite(&self, durees: &DureesSession) -> OffsetDateTime {
        self.derniere_activite + durees.inactivite
    }

    /// Échéance : la plus proche des deux fins.
    #[must_use]
    pub fn echeance(&self, durees: &DureesSession) -> OffsetDateTime {
        self.fin_absolue(durees).min(self.fin_inactivite(durees))
    }

    /// Vrai si la session admet une requête à `maintenant` : non révoquée, ouverte, avant son
    /// échéance. Un instant d'ouverture dans le futur la refuse (refus en cas de doute).
    #[must_use]
    pub fn est_valide(&self, maintenant: OffsetDateTime, durees: &DureesSession) -> bool {
        self.revoquee_le.is_none()
            && self.cree_le <= maintenant
            && maintenant < self.echeance(durees)
    }
}

/// Paramètres d'une nouvelle session.
#[derive(Debug, Clone)]
pub struct NouvelleSession {
    /// Titulaire.
    pub utilisateur: IdUtilisateur,
    /// Nature.
    pub nature: NatureSession,
    /// Facteur vérifié.
    pub methode: MethodeSession,
    /// Instant d'ouverture, qui vaut aussi vérification de facteur.
    pub instant: OffsetDateTime,
    /// Appareil (200 caractères au plus ; tronqué au-delà).
    pub appareil: Option<String>,
    /// Adresse du client.
    pub adresse: Option<std::net::IpAddr>,
}

/// Longueur maximale de la description de l'appareil (contrainte de la table).
pub const APPAREIL_MAX: usize = 200;

/// Une ligne lue (enregistrement de `query!` aux colonnes de la table) en [`Session`].
macro_rules! session_lue {
    ($l:expr) => {{
        let l = $l;
        Ok::<_, ErreurBase>(Session {
            id: IdSession::depuis_uuid(l.id),
            utilisateur: IdUtilisateur::depuis_uuid(l.utilisateur_id),
            nature: NatureSession::depuis_nom(&l.nature)
                .ok_or(ErreurBase::Donnee("nature de session"))?,
            methode: MethodeSession::depuis_nom(&l.methode)
                .ok_or(ErreurBase::Donnee("méthode de session"))?,
            cree_le: l.cree_le,
            derniere_activite: l.derniere_activite,
            derniere_verification: l.derniere_verification,
            revoquee_le: l.revoquee_le,
            appareil: l.appareil,
            adresse: l.adresse,
        })
    }};
}

/// Lit la session dont l'empreinte est présentée, dans une courte transaction qui ne pose que
/// `sigma.jeton_session` : la RLS n'y laisse voir que cette session. Absente si aucune session
/// n'a ce jeton (jeton faux, ou remplacé par une rotation).
///
/// # Errors
/// Erreur de PostgreSQL, ou donnée incohérente.
pub async fn lire_par_empreinte(
    pool: &sqlx::PgPool,
    empreinte: &EmpreinteJeton,
) -> Result<Option<Session>, ErreurBase> {
    let mut tx = pool.begin().await?;
    poser_empreinte(&mut tx, empreinte).await?;
    let ligne = sqlx::query!(
        r#"
        SELECT id, utilisateur_id, nature, methode, cree_le, derniere_activite,
               derniere_verification, revoquee_le, appareil, host(adresse) AS adresse
        FROM session WHERE jeton_hache = $1
        "#,
        empreinte.octets(),
    )
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    ligne.map(|l| session_lue!(l)).transpose()
}

/// Contexte de l'authentification : l'empreinte présentée, rien d'autre (ni cabinet, ni dossier,
/// ni utilisateur).
async fn poser_empreinte(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    empreinte: &EmpreinteJeton,
) -> Result<(), ErreurBase> {
    sqlx::query!(
        "SELECT set_config('sigma.cabinet', '', true) AS cabinet, \
                set_config('sigma.dossier', '', true) AS dossier, \
                set_config('sigma.utilisateur', '', true) AS utilisateur, \
                set_config('sigma.jeton_session', encode($1::bytea, 'hex'), true) AS jeton",
        empreinte.octets(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

/// Crée une session pour l'utilisateur du contexte, puis purge ses sessions closes (expirées ou
/// révoquées) depuis plus de [`DureesSession::conservation`]. La purge ne porte que sur cet
/// utilisateur (filtre de la requête et RLS) : aucune opération ne traverse plusieurs
/// utilisateurs.
///
/// # Errors
/// [`ErreurBase::Donnee`] si la session n'est pas celle de l'utilisateur du contexte, ou erreur
/// de PostgreSQL.
pub async fn creer(
    tx: &mut TransactionUtilisateur,
    id: IdSession,
    nouvelle: &NouvelleSession,
    empreinte: &EmpreinteJeton,
    durees: &DureesSession,
) -> Result<(), ErreurBase> {
    let utilisateur = tx.utilisateur();
    if nouvelle.utilisateur != utilisateur {
        return Err(ErreurBase::Donnee("titulaire de la session"));
    }
    let appareil = nouvelle
        .appareil
        .as_ref()
        .map(|a| a.chars().take(APPAREIL_MAX).collect::<String>());
    let adresse = nouvelle.adresse.map(|a| a.to_string());
    sqlx::query!(
        "INSERT INTO session (id, utilisateur_id, jeton_hache, nature, methode, cree_le, \
                              derniere_activite, derniere_verification, appareil, adresse) \
         VALUES ($1, $2, $3, $4, $5, $6, $6, $6, $7, ($8::text)::inet)",
        id.uuid(),
        utilisateur.uuid(),
        empreinte.octets(),
        nouvelle.nature.nom(),
        nouvelle.methode.nom(),
        nouvelle.instant,
        appareil,
        adresse,
    )
    .execute(tx.connexion())
    .await?;
    purger(tx, nouvelle.instant, durees).await
}

/// Supprime les sessions de l'utilisateur du contexte closes depuis plus de
/// [`DureesSession::conservation`] à `maintenant` : révoquées, ou expirées (ouverture ou
/// dernière activité assez anciennes ; la durée maximale d'une session complète, la plus
/// longue, sert de borne).
async fn purger(
    tx: &mut TransactionUtilisateur,
    maintenant: OffsetDateTime,
    durees: &DureesSession,
) -> Result<(), ErreurBase> {
    let seuil = maintenant - durees.conservation;
    let utilisateur = tx.utilisateur().uuid();
    sqlx::query!(
        "DELETE FROM session WHERE utilisateur_id = $1 \
           AND (revoquee_le < $2 OR cree_le < $3 OR derniere_activite < $4)",
        utilisateur,
        seuil,
        seuil - durees.duree_max,
        seuil - durees.inactivite,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Les sessions de l'utilisateur du contexte, des plus récentes aux plus anciennes (révoquées
/// et expirées comprises : l'appelant filtre).
///
/// # Errors
/// Erreur de PostgreSQL, ou donnée incohérente.
pub async fn lister(tx: &mut TransactionUtilisateur) -> Result<Vec<Session>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let lignes = sqlx::query!(
        r#"
        SELECT id, utilisateur_id, nature, methode, cree_le, derniere_activite,
               derniere_verification, revoquee_le, appareil, host(adresse) AS adresse
        FROM session WHERE utilisateur_id = $1
        ORDER BY cree_le DESC, id
        "#,
        utilisateur,
    )
    .fetch_all(tx.connexion())
    .await?;
    lignes.into_iter().map(|l| session_lue!(l)).collect()
}

/// Une session de l'utilisateur du contexte ; celle d'un autre est absente.
///
/// # Errors
/// Erreur de PostgreSQL, ou donnée incohérente.
pub async fn lire(
    tx: &mut TransactionUtilisateur,
    id: IdSession,
) -> Result<Option<Session>, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let ligne = sqlx::query!(
        r#"
        SELECT id, utilisateur_id, nature, methode, cree_le, derniere_activite,
               derniere_verification, revoquee_le, appareil, host(adresse) AS adresse
        FROM session WHERE id = $1 AND utilisateur_id = $2
        "#,
        id.uuid(),
        utilisateur,
    )
    .fetch_optional(tx.connexion())
    .await?;
    ligne.map(|l| session_lue!(l)).transpose()
}

/// Enregistre l'activité d'une session non révoquée de l'utilisateur du contexte à `instant`,
/// si sa dernière activité est antérieure à `avant` (écriture espacée). Appelée après une
/// requête **admise** (nature, anti-CSRF, décision) : une requête refusée ne prolonge pas la
/// session.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn noter_activite(
    tx: &mut TransactionUtilisateur,
    id: IdSession,
    instant: OffsetDateTime,
    avant: OffsetDateTime,
) -> Result<(), ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    sqlx::query!(
        "UPDATE session SET derniere_activite = $3 \
         WHERE id = $1 AND utilisateur_id = $2 AND revoquee_le IS NULL \
           AND derniere_activite < $4",
        id.uuid(),
        utilisateur,
        instant,
        avant,
    )
    .execute(tx.connexion())
    .await?;
    Ok(())
}

/// Rotation : remplace l'empreinte du jeton d'une session non révoquée de l'utilisateur du
/// contexte, **sur la même ligne** (l'identifiant public, cité par l'audit, ne change pas), à
/// condition que son empreinte soit encore `ancienne`, celle du jeton présenté. Deux rotations
/// concurrentes avec le même jeton : la seconde ne touche aucune ligne (elle relit la ligne
/// après la première, `READ COMMITTED`) et ne peut pas s'approprier la session. Le jeton
/// précédent ne désigne plus rien. Faux si la session est absente, révoquée, ou déjà tournée.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn tourner(
    tx: &mut TransactionUtilisateur,
    id: IdSession,
    ancienne: &EmpreinteJeton,
    nouvelle: &EmpreinteJeton,
) -> Result<bool, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let resultat = sqlx::query!(
        "UPDATE session SET jeton_hache = $3 \
         WHERE id = $1 AND utilisateur_id = $2 AND revoquee_le IS NULL AND jeton_hache = $4",
        id.uuid(),
        utilisateur,
        nouvelle.octets(),
        ancienne.octets(),
    )
    .execute(tx.connexion())
    .await?;
    Ok(resultat.rows_affected() == 1)
}

/// Révoque une session non révoquée de l'utilisateur du contexte. Faux si elle est absente,
/// d'un autre utilisateur, ou déjà révoquée.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn revoquer(
    tx: &mut TransactionUtilisateur,
    id: IdSession,
    instant: OffsetDateTime,
    motif: MotifRevocation,
) -> Result<bool, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let resultat = sqlx::query!(
        "UPDATE session SET revoquee_le = $3, motif_revocation = $4 \
         WHERE id = $1 AND utilisateur_id = $2 AND revoquee_le IS NULL",
        id.uuid(),
        utilisateur,
        instant,
        motif.nom(),
    )
    .execute(tx.connexion())
    .await?;
    Ok(resultat.rows_affected() == 1)
}

/// Révoque toutes les sessions non révoquées de l'utilisateur du contexte, sauf `gardee` (la
/// session de la requête) : changement de facteur (ADR 0012). Rend le nombre de sessions
/// révoquées.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn revoquer_autres(
    tx: &mut TransactionUtilisateur,
    gardee: IdSession,
    instant: OffsetDateTime,
    motif: MotifRevocation,
) -> Result<u64, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let resultat = sqlx::query!(
        "UPDATE session SET revoquee_le = $3, motif_revocation = $4 \
         WHERE utilisateur_id = $1 AND id <> $2 AND revoquee_le IS NULL",
        utilisateur,
        gardee.uuid(),
        instant,
        motif.nom(),
    )
    .execute(tx.connexion())
    .await?;
    Ok(resultat.rows_affected())
}

/// Ré-authentification : `derniere_verification` de la session non révoquée `id` de
/// l'utilisateur du contexte passe à `instant` (session « fraîche », ADR 0012). Faux si elle
/// est absente ou révoquée.
///
/// # Errors
/// Erreur de PostgreSQL.
pub async fn marquer_verification(
    tx: &mut TransactionUtilisateur,
    id: IdSession,
    instant: OffsetDateTime,
) -> Result<bool, ErreurBase> {
    let utilisateur = tx.utilisateur().uuid();
    let resultat = sqlx::query!(
        "UPDATE session SET derniere_verification = $3, derniere_activite = $3 \
         WHERE id = $1 AND utilisateur_id = $2 AND revoquee_le IS NULL",
        id.uuid(),
        utilisateur,
        instant,
    )
    .execute(tx.connexion())
    .await?;
    Ok(resultat.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(nature: NatureSession) -> Session {
        Session {
            id: IdSession::nouveau(),
            utilisateur: IdUtilisateur::nouveau(),
            nature,
            methode: MethodeSession::Passkey,
            cree_le: OffsetDateTime::UNIX_EPOCH,
            derniere_activite: OffsetDateTime::UNIX_EPOCH,
            derniere_verification: OffsetDateTime::UNIX_EPOCH,
            revoquee_le: None,
            appareil: None,
            adresse: None,
        }
    }

    #[test]
    fn echeances_d_une_session() {
        let d = DureesSession::default();
        let t0 = OffsetDateTime::UNIX_EPOCH;
        let mut s = session(NatureSession::Complete);
        assert!(s.est_valide(t0, &d));
        assert!(s.est_valide(t0 + Duration::minutes(29), &d));
        // Inactivité : 30 minutes.
        assert!(!s.est_valide(t0 + Duration::minutes(30), &d));
        // Active, mais au-delà de 12 heures.
        s.derniere_activite = t0 + Duration::hours(12) - Duration::minutes(1);
        assert!(s.est_valide(t0 + Duration::hours(12) - Duration::seconds(1), &d));
        assert!(!s.est_valide(t0 + Duration::hours(12), &d));
        // Ouverte dans le futur : refusée.
        assert!(!s.est_valide(t0 - Duration::seconds(1), &d));
        // Révoquée.
        s.revoquee_le = Some(t0);
        assert!(!s.est_valide(t0 + Duration::minutes(1), &d));
        // Restreinte : 15 minutes au plus, même active.
        let mut r = session(NatureSession::Restreinte);
        r.derniere_activite = t0 + Duration::minutes(14);
        assert!(r.est_valide(t0 + Duration::minutes(14), &d));
        assert!(!r.est_valide(t0 + Duration::minutes(15), &d));
    }

    #[test]
    fn la_configuration_ne_fait_que_raccourcir() {
        let longues = DureesSession {
            inactivite: Duration::hours(2),
            duree_max: Duration::days(2),
            duree_max_enrolement: Duration::hours(1),
            pas_activite: Duration::minutes(10),
            conservation: Duration::days(90),
        }
        .raccourcies();
        let adr = DureesSession::default();
        assert_eq!(longues.inactivite, adr.inactivite);
        assert_eq!(longues.duree_max, adr.duree_max);
        assert_eq!(longues.duree_max_enrolement, adr.duree_max_enrolement);
        assert_eq!(longues.pas_activite, adr.pas_activite);
        assert_eq!(longues.conservation, Duration::days(90));
        let courtes = DureesSession {
            inactivite: Duration::minutes(10),
            ..adr
        }
        .raccourcies();
        assert_eq!(courtes.inactivite, Duration::minutes(10));
    }

    #[test]
    fn noms_enregistres() {
        for n in [
            NatureSession::Complete,
            NatureSession::Restreinte,
            NatureSession::Enrolement,
        ] {
            assert_eq!(NatureSession::depuis_nom(n.nom()), Some(n));
        }
        for m in [
            MethodeSession::Passkey,
            MethodeSession::Secours,
            MethodeSession::CodeRecuperation,
            MethodeSession::LienEnrolement,
        ] {
            assert_eq!(MethodeSession::depuis_nom(m.nom()), Some(m));
        }
        assert_eq!(NatureSession::depuis_nom("inconnue"), None);
        assert!(!format!("{:?}", EmpreinteJeton::new([7; 32])).contains('7'));
    }
}
