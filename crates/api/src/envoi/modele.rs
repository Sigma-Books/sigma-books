//! Modèles des courriels (ADR 0012, F12.2) : **constants et sobres**. Aucun montant, aucune
//! pièce, aucun nom de tiers ni de dossier, aucun nom de cabinet : seulement ce qu'il faut pour
//! que le destinataire agisse, et un lien vers **le seul domaine de l'instance**.
//!
//! Le sujet est constant. Le nom du destinataire (saisi par un administrateur, validé par
//! [`crate::courriel::nom`]) n'apparaît que dans le **corps**, jamais dans un en-tête : même un
//! nom que la validation laisserait passer ne pourrait pas injecter d'en-tête. Le lien est dans
//! le fragment de l'adresse (`/lien#…`) : il n'est transmis ni au serveur ni dans `Referer`.
//! Jamais de mot de passe dans un courriel (ADR 0012).

use crate::courriel;

/// Un courriel prêt à être remis au transport.
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    /// Adresse du destinataire (normalisée, validée).
    pub destinataire: String,
    /// Sujet (constant).
    pub sujet: &'static str,
    /// Corps, en texte brut.
    pub texte: String,
}

impl std::fmt::Debug for Message {
    /// Le corps contient un lien secret : il n'est jamais affiché.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("sujet", &self.sujet)
            .finish_non_exhaustive()
    }
}

/// Adresse du lien : l'origine de l'instance, puis le jeton dans le fragment.
#[must_use]
pub fn adresse_du_lien(origine: &str, jeton: &str) -> String {
    format!("{}/lien#{jeton}", origine.trim_end_matches('/'))
}

/// Sujet de l'invitation.
pub const SUJET_INVITATION: &str = "Invitation à rejoindre Sigma Books";
/// Sujet de la réinitialisation d'accès.
pub const SUJET_REINITIALISATION: &str = "Réinitialisation de votre accès à Sigma Books";

/// Invitation à rejoindre un cabinet (lien valable 24 heures).
///
/// `None` si l'adresse ou le nom ne passent pas la validation (jamais d'envoi à une adresse
/// douteuse, même lue en base).
#[must_use]
pub fn invitation(destinataire: &str, nom: &str, lien: &str) -> Option<Message> {
    let destinataire = courriel::adresse(destinataire)?;
    let nom = courriel::nom(nom)?;
    Some(Message {
        destinataire,
        sujet: SUJET_INVITATION,
        texte: format!(
            "Bonjour {nom},\n\n\
             Un administrateur vous invite à rejoindre un cabinet sur Sigma Books.\n\
             Pour accepter l'invitation et enregistrer votre clé d'accès (passkey), ouvrez ce lien \
             dans votre navigateur :\n\n{lien}\n\n\
             Ce lien est personnel, ne fonctionne qu'une fois et expire dans 24 heures. Ne le \
             transmettez à personne.\n\
             Si vous n'attendiez pas cette invitation, ignorez ce message : sans ce lien, rien ne \
             se passe.\n"
        ),
    })
}

/// Réinitialisation d'accès décidée par un administrateur, au terme du délai de 24 heures (lien
/// valable une heure).
#[must_use]
pub fn reinitialisation(destinataire: &str, nom: &str, lien: &str) -> Option<Message> {
    let destinataire = courriel::adresse(destinataire)?;
    let nom = courriel::nom(nom)?;
    Some(Message {
        destinataire,
        sujet: SUJET_REINITIALISATION,
        texte: format!(
            "Bonjour {nom},\n\n\
             Un administrateur de votre cabinet a demandé la réinitialisation de votre accès à \
             Sigma Books, et le délai de contestation de 24 heures est écoulé.\n\
             Pour enregistrer une nouvelle clé d'accès (passkey), ouvrez ce lien dans votre \
             navigateur :\n\n{lien}\n\n\
             Ce lien est personnel, ne fonctionne qu'une fois et expire dans une heure. Vos \
             anciens moyens de connexion seront retirés à son usage. Ne le transmettez à personne.\n\
             Si vous n'êtes pas à l'origine de cette demande, ne l'ouvrez pas et prévenez un \
             administrateur de votre cabinet.\n"
        ),
    })
}

/// À qui s'adresse une alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destinataire {
    /// Administrateur actif du cabinet.
    Administrateur,
    /// Membre concerné par l'événement (réinitialisation de son accès).
    Concerne,
}

/// Ce que le témoin remis au cabinet cite : des empreintes et des nombres, jamais une donnée
/// comptable (`docs/integrite/format-v2.md` §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemoinAlerte {
    /// Nombre d'écritures chaînées.
    pub nombre: i64,
    /// Empreinte de tête de la chaîne (hexadécimal).
    pub tete: String,
    /// Jeton déjà obtenu : (nombre d'écritures couvert, instant d'obtention en UTC).
    pub jeton: Option<(i64, String)>,
}

/// Alerte de sécurité ou de suivi (ADR 0012, 0022, 0025) : **texte constant** par sujet. Aucun
/// nom, aucun montant ; seul un identifiant de dossier (UUID) et, pour le témoin, des empreintes
/// et des nombres. Le sujet de la notification n'est pas un en-tête libre : il vient d'une liste
/// fermée.
#[must_use]
#[allow(clippy::too_many_lines)] // Un texte constant par sujet : une table, pas de logique.
pub fn alerte(
    sujet: crate::base::taches::SujetNotification,
    destinataire: &str,
    nom: &str,
    qui: Destinataire,
    dossier: Option<uuid::Uuid>,
    temoin: Option<&TemoinAlerte>,
) -> Option<Message> {
    use crate::base::taches::SujetNotification as S;
    let destinataire = courriel::adresse(destinataire)?;
    let nom = courriel::nom(nom)?;
    let dossier_texte = dossier.map_or_else(String::new, |d| format!("\nDossier : {d}"));
    let (sujet_courriel, corps): (&'static str, String) = match (sujet, qui) {
        (S::ExportAuditCabinet, _) => (
            "Sigma Books : export de l'audit du cabinet",
            "L'audit de votre cabinet vient d'être exporté.".to_owned(),
        ),
        (S::ExportAuditDossier, _) => (
            "Sigma Books : export de l'audit d'un dossier",
            format!("L'audit d'un dossier de votre cabinet vient d'être exporté.{dossier_texte}"),
        ),
        (S::ExportIntegrite, _) => (
            "Sigma Books : export d'intégrité d'un dossier",
            format!(
                "Un export d'intégrité d'un dossier de votre cabinet vient d'être réalisé.{dossier_texte}"
            ),
        ),
        (S::RetardHorodatage, _) => (
            "Sigma Books : horodatage en retard",
            format!(
                "Aucun horodatage n'a pu être obtenu depuis plus de 24 heures pour un dossier de \
                 votre cabinet. Le serveur continue d'essayer ; vérifiez l'accès sortant aux \
                 autorités d'horodatage.{dossier_texte}"
            ),
        ),
        (S::Temoin, _) => {
            let faits = temoin.map_or_else(
                || "Le témoin n'est pas disponible pour le moment.".to_owned(),
                |t| {
                    let jeton = t.jeton.as_ref().map_or_else(
                        || "Aucun jeton d'horodatage n'est encore obtenu pour cette chaîne.".to_owned(),
                        |(n, quand)| format!("Dernier jeton d'horodatage : {n} écriture(s) couverte(s), obtenu le {quand} (UTC)."),
                    );
                    format!(
                        "Écritures chaînées : {}\nEmpreinte de tête : {}\n{jeton}",
                        t.nombre, t.tete
                    )
                },
            );
            (
                "Sigma Books : témoin de la chaîne d'un dossier",
                format!(
                    "Une période a été verrouillée ou clôturée. Conservez ce témoin hors de \
                     Sigma Books : il permet de vérifier plus tard que la chaîne n'a pas été \
                     refaite.{dossier_texte}\n{faits}"
                ),
            )
        }
        (S::ReinitialisationDemandee, Destinataire::Concerne) => (
            "Sigma Books : réinitialisation de votre accès demandée",
            "Un administrateur de votre cabinet a demandé la réinitialisation de votre accès. \
             Votre accès actuel reste valable pendant 24 heures. Si vous n'êtes pas à l'origine \
             ou au courant de cette demande, connectez-vous et contestez-la dans Sigma Books, ou \
             prévenez un autre administrateur."
                .to_owned(),
        ),
        (S::ReinitialisationDemandee, Destinataire::Administrateur) => (
            "Sigma Books : réinitialisation d'un accès demandée",
            "La réinitialisation de l'accès d'un membre de votre cabinet a été demandée. Elle \
             aboutit dans 24 heures, sauf annulation par un administrateur ou par le membre. \
             Si vous ne la reconnaissez pas, annulez-la dans l'administration du cabinet."
                .to_owned(),
        ),
        (S::ReinitialisationAnnulee, _) => (
            "Sigma Books : réinitialisation d'un accès annulée",
            "La réinitialisation de l'accès d'un membre de votre cabinet a été annulée.".to_owned(),
        ),
        (S::ReinitialisationAboutie, _) => (
            "Sigma Books : accès d'un membre réinitialisé",
            "Le lien de réinitialisation de l'accès d'un membre de votre cabinet vient d'être \
             utilisé : ses anciens moyens de connexion sont retirés. Si vous ne reconnaissez pas \
             cette opération, suspendez le membre dans l'administration du cabinet."
                .to_owned(),
        ),
        (S::DroitsModifies, _) => (
            "Sigma Books : droits modifiés dans votre cabinet",
            "Les droits d'un membre de votre cabinet ont changé (rôle, suspension, accès aux \
             dossiers, ou arrivée d'un membre). Consultez l'audit du cabinet pour le détail."
                .to_owned(),
        ),
        (S::SeparationDesactivee, _) => (
            "Sigma Books : séparation des tâches désactivée",
            "La séparation des tâches de votre cabinet a été désactivée : un utilisateur peut \
             désormais valider ses propres saisies."
                .to_owned(),
        ),
        (S::ConnexionSecours, _) => (
            "Sigma Books : connexion par la méthode de secours",
            "Un membre de votre cabinet vient de se connecter par la méthode de secours (mot de \
             passe et code), non par une passkey. Si ce n'était pas lui, suspendez-le."
                .to_owned(),
        ),
        (S::CodeRecuperationUtilise, _) => (
            "Sigma Books : code de récupération utilisé",
            "Un membre de votre cabinet vient d'utiliser un code de récupération. Si ce n'était \
             pas lui, suspendez-le et réinitialisez son accès."
                .to_owned(),
        ),
    };
    Some(Message {
        destinataire,
        sujet: sujet_courriel,
        texte: format!(
            "Bonjour {nom},\n\n{corps}\n\nCe message est automatique ; il ne contient aucune donnée comptable.\n"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lien jamais écrit en dur : un jeton tiré à chaque appel.
    fn lien() -> String {
        let jeton = crate::base::liens::JetonLien::tirer().unwrap().texte();
        adresse_du_lien("https://compta.exemple.sn", &jeton)
    }

    #[test]
    fn le_lien_est_dans_le_fragment_de_l_origine() {
        assert_eq!(
            adresse_du_lien("https://compta.exemple.sn/", "JETON"),
            "https://compta.exemple.sn/lien#JETON"
        );
    }

    #[test]
    fn l_invitation_ne_contient_que_le_necessaire() {
        let lien = lien();
        let m = invitation("Awa@Cabinet.SN", "Awa Diop", &lien).unwrap();
        assert_eq!(m.destinataire, "awa@cabinet.sn");
        assert_eq!(m.sujet, SUJET_INVITATION);
        assert!(m.texte.contains(&lien) && m.texte.contains("Awa Diop"));
        assert!(m.texte.contains("24 heures") && m.texte.contains("ignorez ce message"));
        // Ni mot de passe, ni montant, ni cabinet.
        for interdit in ["mot de passe :", "FCFA", "XOF", "dossier", "cabinet X"] {
            assert!(!m.texte.contains(interdit), "{interdit}");
        }
    }

    #[test]
    fn la_reinitialisation_annonce_une_heure_et_le_retrait_des_anciens_facteurs() {
        let lien = lien();
        let m = reinitialisation("awa@cabinet.sn", "Awa Diop", &lien).unwrap();
        assert_eq!(m.sujet, SUJET_REINITIALISATION);
        assert!(m.texte.contains("une heure") && m.texte.contains("retirés"));
    }

    #[test]
    fn une_adresse_ou_un_nom_douteux_ne_produit_aucun_message() {
        let lien = lien();
        assert!(invitation("a@b.sn\nBcc: x@y.sn", "Nom", &lien).is_none());
        assert!(invitation("a@cabinet.sn", "Nom\r\nSubject: faux", &lien).is_none());
        assert!(reinitialisation("pas une adresse", "Nom", &lien).is_none());
    }

    #[test]
    fn le_debug_ne_montre_pas_le_lien() {
        let lien = lien();
        let m = invitation("awa@cabinet.sn", "Awa", &lien).unwrap();
        let debug = format!("{m:?}");
        assert!(
            !debug.contains("lien#") && !debug.contains("awa@cabinet.sn"),
            "{debug}"
        );
    }

    #[test]
    fn le_sujet_est_constant_quel_que_soit_le_destinataire() {
        let lien = lien();
        let a = invitation("a@cabinet.sn", "Un", &lien).unwrap();
        let b = invitation("b@cabinet.sn", "Deux", &lien).unwrap();
        assert_eq!(a.sujet, b.sujet);
    }

    #[test]
    fn chaque_alerte_est_constante_et_sans_donnee_personnelle() {
        use crate::base::taches::SujetNotification as S;
        let mut sujets = std::collections::BTreeSet::new();
        for sujet in S::TOUS {
            for qui in [Destinataire::Administrateur, Destinataire::Concerne] {
                let m = alerte(sujet, "awa@cabinet.sn", "Awa Diop", qui, None, None).unwrap();
                assert!(m.sujet.is_ascii() || m.sujet.chars().all(|c| !c.is_control()));
                assert!(!m.sujet.contains('\n') && !m.sujet.contains("Awa"));
                assert!(!m.texte.contains("FCFA") && !m.texte.contains("XOF"));
                sujets.insert((m.sujet, qui == Destinataire::Concerne));
            }
        }
        assert!(sujets.len() >= S::TOUS.len());
    }

    #[test]
    fn le_temoin_cite_des_empreintes_et_des_nombres() {
        use crate::base::taches::SujetNotification as S;
        let t = TemoinAlerte {
            nombre: 42,
            tete: "ab".repeat(32),
            jeton: Some((42, "2026-11-25T10:00:00Z".to_owned())),
        };
        let d = uuid::Uuid::nil();
        let m = alerte(
            S::Temoin,
            "a@cabinet.sn",
            "A",
            Destinataire::Administrateur,
            Some(d),
            Some(&t),
        )
        .unwrap();
        assert!(m.texte.contains("Écritures chaînées : 42") && m.texte.contains(&t.tete));
        assert!(m.texte.contains(&d.to_string()));
    }

    #[test]
    fn une_alerte_a_une_adresse_douteuse_ne_part_pas() {
        use crate::base::taches::SujetNotification as S;
        assert!(
            alerte(
                S::DroitsModifies,
                "a@b.sn\nBcc: x@y.sn",
                "A",
                Destinataire::Administrateur,
                None,
                None
            )
            .is_none()
        );
        assert!(
            alerte(
                S::DroitsModifies,
                "a@cabinet.sn",
                "A\r\nB",
                Destinataire::Administrateur,
                None,
                None
            )
            .is_none()
        );
    }
}
