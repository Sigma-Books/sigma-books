//! Commandes d'administration de **l'instance** (ADR 0012, question 4a ; PR 9a), exécutées sur le
//! serveur par l'exploitation, jamais par une route HTTP :
//!
//! - `api creer-cabinet --nom <nom> --administrateur-courriel <adresse> --administrateur-nom
//!   <nom>` : crée un cabinet et invite son premier administrateur. Pas d'inscription libre ;
//! - `api reinitialiser-acces --cabinet <uuid> --courriel <adresse> --motif-code <code> --motif <texte>` : procédure
//!   de support pour un administrateur **sans autre administrateur** dans son cabinet (ADR 0012) :
//!   même délai de 24 heures et mêmes traces que la demande d'un administrateur. L'identité du
//!   demandeur est vérifiée **hors bande** par l'exploitation avant de lancer la commande.
//!
//! Chaque commande inscrit une entrée à l'audit du cabinet (acteur « système »). Le lien
//! d'invitation est écrit **une seule fois** sur la sortie standard (jamais dans les journaux
//! techniques ni en base) : l'exploitant le transmet hors bande. Avec un relais SMTP configuré
//! (PR 9b), l'envoi par courriel prend le relais. Les arguments sont validés comme les saisies
//! HTTP (adresses sans retour à la ligne : [`crate::courriel`]).

use integrite::audit::{Acteur, Evenement, Ressource, Symbole, Texte, Uuid as UuidAudit};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::base::administration::{
    CreationReinitialisation, DELAI_REINITIALISATION, MOTIFS_REINITIALISATION, NouvelleInvitation,
    appartient_a_d_autres_cabinets, creer_invitation, creer_reinitialisation, lister_membres,
};
use crate::base::audit::{PorteeAudit, inscrire};
use crate::base::liens::{self, NatureLien};
use crate::base::taches::{SujetCourriel, planifier_courriel};
use crate::base::{
    ContexteTenant, ErreurBase, IdCabinet, IdUtilisateur, TransactionTenant, ouvrir_transaction,
};
use crate::courriel;
use crate::envoi::modele;

/// Commande d'administration de l'instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Commande {
    /// Créer un cabinet et inviter son premier administrateur.
    CreerCabinet {
        /// Nom du cabinet.
        nom: String,
        /// Courriel normalisé du premier administrateur.
        courriel: String,
        /// Nom du premier administrateur.
        nom_administrateur: String,
    },
    /// Réinitialiser l'accès d'un administrateur sans autre administrateur.
    ReinitialiserAcces {
        /// Cabinet.
        cabinet: IdCabinet,
        /// Courriel normalisé du membre.
        courriel: String,
        /// Code du motif (liste fermée) : seul inscrit à l'audit chaîné.
        motif_code: String,
        /// Motif libre (vérification d'identité hors bande effectuée), conservé dans la table.
        motif: String,
    },
}

/// Erreur d'une commande : message sobre pour l'exploitant, jamais de secret.
#[derive(Debug, thiserror::Error)]
pub enum ErreurCommande {
    /// Arguments invalides.
    #[error("{0}")]
    Usage(String),
    /// Refus métier (membre absent, invitation en cours…).
    #[error("{0}")]
    Refus(&'static str),
    /// Erreur d'accès à la base.
    #[error("base de données : {0}")]
    Base(#[from] ErreurBase),
}

const USAGE: &str = "usage : api creer-cabinet --nom <nom> --administrateur-courriel <adresse> \
                     --administrateur-nom <nom> | api reinitialiser-acces --cabinet <uuid> \
                     --courriel <adresse> --motif-code <code> --motif <texte>";

fn usage<T>() -> Result<T, ErreurCommande> {
    Err(ErreurCommande::Usage(USAGE.to_owned()))
}

/// Lit les arguments après le programme. `Ok(None)` : aucun argument, le serveur démarre.
///
/// # Errors
/// [`ErreurCommande::Usage`] : commande inconnue, option inconnue, répétée, manquante ou invalide.
pub fn lire_commande(args: &[String]) -> Result<Option<Commande>, ErreurCommande> {
    let Some((commande, options)) = args.split_first() else {
        return Ok(None);
    };
    let mut valeurs: Vec<(&str, &str)> = Vec::new();
    let mut it = options.iter();
    while let Some(cle) = it.next() {
        let Some(valeur) = it.next() else {
            return usage();
        };
        if !cle.starts_with("--") || valeurs.iter().any(|(k, _)| k == cle) {
            return usage();
        }
        valeurs.push((cle, valeur));
    }
    let option = |nom: &str| valeurs.iter().find(|(k, _)| *k == nom).map(|(_, v)| *v);
    let connues = |noms: &[&str]| valeurs.iter().all(|(k, _)| noms.contains(k));
    match commande.as_str() {
        "creer-cabinet"
            if connues(&["--nom", "--administrateur-courriel", "--administrateur-nom"]) =>
        {
            let (Some(nom), Some(c), Some(n)) = (
                option("--nom"),
                option("--administrateur-courriel"),
                option("--administrateur-nom"),
            ) else {
                return usage();
            };
            match (courriel::nom(nom), courriel::adresse(c), courriel::nom(n)) {
                (Some(nom), Some(courriel), Some(nom_administrateur)) => {
                    Ok(Some(Commande::CreerCabinet {
                        nom,
                        courriel,
                        nom_administrateur,
                    }))
                }
                _ => usage(),
            }
        }
        "reinitialiser-acces"
            if connues(&["--cabinet", "--courriel", "--motif-code", "--motif"]) =>
        {
            let (Some(cabinet), Some(c), Some(code), Some(motif)) = (
                option("--cabinet"),
                option("--courriel"),
                option("--motif-code"),
                option("--motif"),
            ) else {
                return usage();
            };
            let Some(code) = MOTIFS_REINITIALISATION.iter().find(|m| **m == code) else {
                return usage();
            };
            let cabinet = Uuid::try_parse(cabinet)
                .ok()
                .filter(|u| !u.is_nil() && !u.is_max());
            let motif = motif.trim();
            match (cabinet, courriel::adresse(c)) {
                (Some(cabinet), Some(courriel))
                    if !motif.is_empty()
                        && motif.chars().count() <= 500
                        && !motif.chars().any(char::is_control) =>
                {
                    Ok(Some(Commande::ReinitialiserAcces {
                        cabinet: IdCabinet::depuis_uuid(cabinet),
                        courriel,
                        motif_code: (*code).to_owned(),
                        motif: motif.to_owned(),
                    }))
                }
                _ => usage(),
            }
        }
        _ => usage(),
    }
}

fn evenement(
    action: &str,
    genre: &str,
    id: Uuid,
    donnee: (&str, &str),
) -> Result<Evenement, ErreurBase> {
    let invalide = |_| ErreurBase::Audit("événement de commande d'instance");
    Evenement::new(
        Acteur::Systeme,
        Symbole::parse(action).map_err(invalide)?,
        Ressource {
            genre: Symbole::parse(genre).map_err(invalide)?,
            id: UuidAudit::depuis_octets(*id.as_bytes()),
        },
    )
    .avec_donnee(
        Symbole::parse(donnee.0).map_err(invalide)?,
        Texte::parse(donnee.1).map_err(invalide)?,
    )
    .map_err(invalide)
}

/// Résultat d'une commande à afficher à l'exploitant.
#[derive(Debug)]
pub struct Sortie {
    /// Lignes pour l'exploitant (jamais journalisées).
    pub lignes: Vec<String>,
}

/// Exécute une commande sur le pool du rôle applicatif ; `origine` sert à composer le lien.
/// `par_courriel` : un relais SMTP est configuré, le serveur envoie le lien par courriel et la
/// commande n'écrit **aucun** lien ; sinon le lien d'invitation est écrit une fois sur la sortie.
///
/// # Errors
/// [`ErreurCommande`].
pub async fn executer(
    pool: &PgPool,
    commande: &Commande,
    origine: &str,
    instant: OffsetDateTime,
    par_courriel: bool,
) -> Result<Sortie, ErreurCommande> {
    match commande {
        Commande::CreerCabinet {
            nom,
            courriel,
            nom_administrateur,
        } => {
            creer_cabinet(
                pool,
                nom,
                courriel,
                nom_administrateur,
                origine,
                instant,
                par_courriel,
            )
            .await
        }
        Commande::ReinitialiserAcces {
            cabinet,
            courriel,
            motif_code,
            motif,
        } => reinitialiser(pool, *cabinet, courriel, motif_code, motif, instant).await,
    }
}

async fn creer_cabinet(
    pool: &PgPool,
    nom: &str,
    courriel: &str,
    nom_administrateur: &str,
    origine: &str,
    instant: OffsetDateTime,
    par_courriel: bool,
) -> Result<Sortie, ErreurCommande> {
    let cabinet = IdCabinet::nouveau();
    // Acteur fictif pour le contexte : l'action est celle de l'instance, sans utilisateur.
    let mut tx = ouvrir_transaction(
        pool,
        ContexteTenant::cabinet(cabinet, IdUtilisateur::nouveau()),
    )
    .await?;
    sqlx::query!(
        "INSERT INTO cabinet (id, nom) VALUES ($1, $2)",
        cabinet.uuid(),
        nom
    )
    .execute(tx.connexion())
    .await
    .map_err(ErreurBase::from)?;
    let invitation = Uuid::new_v4();
    creer_invitation(
        &mut tx,
        &NouvelleInvitation {
            id: invitation,
            courriel,
            nom: nom_administrateur,
            role: "administrateur",
            invite_par: None,
            instant,
        },
    )
    .await?;
    inscrire(
        &mut tx,
        PorteeAudit::Cabinet,
        instant,
        evenement(
            "CreerCabinet",
            "cabinet",
            cabinet.uuid(),
            ("role_invite", "administrateur"),
        )?,
    )
    .await?;
    let mut lignes = vec![
        format!("cabinet : {cabinet}"),
        format!("invitation : {invitation}"),
    ];
    if par_courriel {
        // Le serveur émet le lien et l'envoie : rien à transmettre ici.
        planifier_courriel(&mut tx, SujetCourriel::Invitation, invitation, None).await?;
        lignes.push("invitation envoyée par courriel par le serveur".to_owned());
    } else {
        let jeton = liens::emettre(&mut tx, NatureLien::Invitation, invitation, instant).await?;
        lignes.push(
            "lien d'invitation (valable 24 h, usage unique, affiché une seule fois ; à transmettre hors bande) :"
                .to_owned(),
        );
        lignes.push(modele::adresse_du_lien(origine, &jeton.texte()));
    }
    tx.confirmer().await?;
    Ok(Sortie { lignes })
}

async fn reinitialiser(
    pool: &PgPool,
    cabinet: IdCabinet,
    courriel: &str,
    motif_code: &str,
    motif: &str,
    instant: OffsetDateTime,
) -> Result<Sortie, ErreurCommande> {
    let mut tx: TransactionTenant = ouvrir_transaction(
        pool,
        ContexteTenant::cabinet(cabinet, IdUtilisateur::nouveau()),
    )
    .await?;
    let membre = lister_membres(&mut tx)
        .await?
        .into_iter()
        .find(|m| m.courriel.eq_ignore_ascii_case(courriel))
        .ok_or(ErreurCommande::Refus(
            "aucun membre de ce cabinet pour ce courriel",
        ))?;
    // Procédure de support : elle ne vaut que pour un compte propre à ce cabinet (ADR 0023, §4).
    // Un compte de plusieurs cabinets relève d'un futur administrateur d'instance.
    if appartient_a_d_autres_cabinets(&mut tx, membre.utilisateur).await? {
        return Err(ErreurCommande::Refus(
            "ce compte appartient à d'autres cabinets : réinitialisation refusée",
        ));
    }
    let id = Uuid::new_v4();
    let issue = creer_reinitialisation(
        &mut tx,
        id,
        membre.utilisateur,
        None,
        motif_code,
        motif,
        instant,
    )
    .await?;
    if issue == CreationReinitialisation::EnCours {
        return Err(ErreurCommande::Refus(
            "une réinitialisation est déjà en attente",
        ));
    }
    // Le lien (1 h) est émis et envoyé au terme du délai par le serveur.
    planifier_courriel(
        &mut tx,
        SujetCourriel::Reinitialisation,
        id,
        Some(instant + DELAI_REINITIALISATION),
    )
    .await?;
    let evenement = evenement(
        "ReinitialiserAcces",
        "reinitialisation",
        id,
        ("membre", &membre.utilisateur.to_string()),
    )?;
    inscrire(&mut tx, PorteeAudit::Cabinet, instant, evenement).await?;
    tx.confirmer().await?;
    Ok(Sortie {
        lignes: vec![
            format!("réinitialisation : {id}"),
            "délai de contestation : 24 heures ; le lien d'enrôlement est émis au terme du délai"
                .to_owned(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn sans_argument_le_serveur_demarre() {
        assert_eq!(lire_commande(&[]).unwrap(), None);
    }

    #[test]
    fn creer_cabinet_se_lit_et_se_normalise() {
        let c = lire_commande(&args(&[
            "creer-cabinet",
            "--nom",
            "Cabinet Diop",
            "--administrateur-courriel",
            "Awa@Cabinet.SN",
            "--administrateur-nom",
            "Awa Diop",
        ]))
        .unwrap();
        assert_eq!(
            c,
            Some(Commande::CreerCabinet {
                nom: "Cabinet Diop".into(),
                courriel: "awa@cabinet.sn".into(),
                nom_administrateur: "Awa Diop".into()
            })
        );
    }

    #[test]
    fn les_arguments_hostiles_ou_incomplets_sont_refuses() {
        let faux: &[&[&str]] = &[
            &["inconnue"],
            &["creer-cabinet"],
            &["creer-cabinet", "--nom", "X"],
            &[
                "creer-cabinet",
                "--nom",
                "X",
                "--nom",
                "Y",
                "--administrateur-courriel",
                "a@b.sn",
                "--administrateur-nom",
                "Z",
            ],
            &[
                "creer-cabinet",
                "--nom",
                "X",
                "--administrateur-courriel",
                "a@b.sn\nBcc: x@y.sn",
                "--administrateur-nom",
                "Z",
            ],
            &[
                "creer-cabinet",
                "--nom",
                "X\r\nY",
                "--administrateur-courriel",
                "a@b.sn",
                "--administrateur-nom",
                "Z",
            ],
            &[
                "creer-cabinet",
                "--nom",
                "X",
                "--administrateur-courriel",
                "a@b.sn",
                "--administrateur-nom",
                "Z",
                "--autre",
                "1",
            ],
            &[
                "reinitialiser-acces",
                "--cabinet",
                "pas-un-uuid",
                "--courriel",
                "a@b.sn",
                "--motif-code",
                "autre",
                "--motif",
                "m",
            ],
            &[
                "reinitialiser-acces",
                "--cabinet",
                "00000000-0000-0000-0000-000000000000",
                "--courriel",
                "a@b.sn",
                "--motif-code",
                "autre",
                "--motif",
                "m",
            ],
            &[
                "reinitialiser-acces",
                "--cabinet",
                "6f1d3c2e-5a47-4b8e-9c10-2d7e8a9b0c11",
                "--courriel",
                "a@b.sn",
                "--motif-code",
                "autre",
                "--motif",
                "  ",
            ],
            &[
                "reinitialiser-acces",
                "--cabinet",
                "6f1d3c2e-5a47-4b8e-9c10-2d7e8a9b0c11",
                "--courriel",
                "a@b.sn",
            ],
        ];
        for a in faux {
            assert!(lire_commande(&args(a)).is_err(), "{a:?}");
        }
    }

    #[test]
    fn reinitialiser_se_lit() {
        let c = lire_commande(&args(&[
            "reinitialiser-acces",
            "--cabinet",
            "6f1d3c2e-5a47-4b8e-9c10-2d7e8a9b0c11",
            "--courriel",
            "A@B.sn",
            "--motif-code",
            "autre",
            "--motif",
            " Appel de vérification ",
        ]))
        .unwrap();
        assert!(matches!(
            c,
            Some(Commande::ReinitialiserAcces { ref courriel, ref motif, .. })
                if courriel == "a@b.sn" && motif == "Appel de vérification"
        ));
    }
}
