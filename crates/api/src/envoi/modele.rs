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
}
