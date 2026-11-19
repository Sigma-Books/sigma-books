//! Chaîne d'audit d'une portée et sa vérification (ADR 0014, « Chaînage »).
//!
//! ```text
//! empreinte_0 = SHA-256("sigma-books audit v1\n" ‖ portée ‖ "=" ‖ identifiant)
//! empreinte_n = SHA-256(empreinte_{n-1} ‖ entrée canonique n)
//! ```
//!
//! `portée` vaut `cabinet` ou `dossier` ; l'identifiant est l'UUID du cabinet ou du dossier, sous
//! sa forme canonique. L'empreinte de départ dépend donc de la portée et de l'identifiant : une
//! chaîne ne peut être greffée ni sur une autre portée ni sur un autre cabinet ou dossier. Chaque
//! entrée répète en outre sa portée, son cabinet, son dossier et son rang, contrôlés à la
//! vérification.

use std::fmt;

use crate::chaine::{Empreinte, chainer};

use super::entree::{EntreeAudit, ErreurAudit, Portee};

/// Préfixe de l'empreinte de départ, version 1 de l'audit.
const GENESE: &[u8] = b"sigma-books audit v1\n";

/// Empreinte de départ de la chaîne d'une portée.
#[must_use]
pub fn genese(portee: &Portee) -> Empreinte {
    let identifiant = portee.identifiant().to_string();
    Empreinte::sha256(&[
        GENESE,
        portee.nom().as_bytes(),
        b"=",
        identifiant.as_bytes(),
    ])
}

/// Chaîne d'audit d'une portée en cours de construction : tête et nombre d'entrées.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaineAudit {
    portee: Portee,
    tete: Empreinte,
    longueur: u64,
}

/// Refus d'ajouter une entrée à une chaîne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErreurAjout {
    /// L'entrée relève d'une autre portée (autre cabinet, autre dossier, autre portée).
    #[error("entrée d'une autre portée")]
    PorteeDifferente,
    /// L'entrée n'a pas le rang suivant de la chaîne.
    #[error("rang {trouve} au lieu de {attendu}")]
    Rang {
        /// Rang suivant de la chaîne.
        attendu: u64,
        /// Rang de l'entrée.
        trouve: u64,
    },
}

impl ChaineAudit {
    /// Chaîne vide : la tête est l'empreinte de départ.
    #[must_use]
    pub fn ouvrir(portee: Portee) -> Self {
        Self {
            tete: genese(&portee),
            portee,
            longueur: 0,
        }
    }

    /// Reprend une chaîne à partir de sa tête enregistrée.
    #[must_use]
    pub fn reprendre(portee: Portee, tete: Empreinte, longueur: u64) -> Self {
        Self {
            portee,
            tete,
            longueur,
        }
    }

    /// Ajoute une entrée, qui doit être de la portée et du rang suivant ; renvoie son contenu
    /// canonique et son empreinte (nouvelle tête).
    ///
    /// # Errors
    /// [`ErreurAjout`].
    pub fn ajouter(&mut self, entree: &EntreeAudit) -> Result<(String, Empreinte), ErreurAjout> {
        if *entree.portee() != self.portee {
            return Err(ErreurAjout::PorteeDifferente);
        }
        let attendu = self.longueur + 1;
        if entree.rang() != attendu {
            return Err(ErreurAjout::Rang {
                attendu,
                trouve: entree.rang(),
            });
        }
        let contenu = entree.serialiser();
        self.tete = chainer(&self.tete, contenu.as_bytes());
        self.longueur = attendu;
        Ok((contenu, self.tete))
    }

    /// Rang de la prochaine entrée.
    #[must_use]
    pub fn rang_suivant(&self) -> u64 {
        self.longueur + 1
    }

    /// Empreinte de tête.
    #[must_use]
    pub fn tete(&self) -> &Empreinte {
        &self.tete
    }

    /// Nombre d'entrées.
    #[must_use]
    pub fn longueur(&self) -> u64 {
        self.longueur
    }

    /// La portée.
    #[must_use]
    pub fn portee(&self) -> &Portee {
        &self.portee
    }
}

/// Anomalie constatée par la vérification d'une chaîne d'audit. `position` compte les entrées
/// à partir de 1 : c'est le rang que l'entrée devrait porter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnomalieAudit {
    /// L'entrée n'est pas une entrée canonique lisible.
    EntreeIllisible {
        /// Position de l'entrée.
        position: u64,
        /// Défaut constaté.
        erreur: ErreurAudit,
    },
    /// L'entrée relève d'une autre portée, d'un autre cabinet ou d'un autre dossier.
    PorteeDifferente {
        /// Position de l'entrée.
        position: u64,
    },
    /// Le rang de l'entrée (ou celui enregistré à côté d'elle) n'est pas sa position : entrée
    /// supprimée, insérée ou déplacée.
    RangDifferent {
        /// Position de l'entrée.
        position: u64,
        /// Rang trouvé.
        trouve: u64,
    },
    /// L'empreinte déclarée n'est pas celle de l'entrée chaînée à la précédente : entrée ou
    /// empreinte altérée.
    EmpreinteAlteree {
        /// Position de l'entrée.
        position: u64,
    },
    /// La tête déclarée n'est pas celle de la chaîne recalculée depuis l'empreinte de départ.
    TeteDifferente,
    /// Le nombre d'entrées déclaré n'est pas le nombre lu.
    NombreDifferent {
        /// Nombre déclaré.
        declare: u64,
        /// Nombre lu.
        lu: u64,
    },
}

impl fmt::Display for AnomalieAudit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntreeIllisible { position, erreur } => {
                write!(f, "entrée {position} illisible : {erreur}")
            }
            Self::PorteeDifferente { position } => {
                write!(f, "entrée {position} d'une autre portée")
            }
            Self::RangDifferent { position, trouve } => {
                write!(
                    f,
                    "entrée {position} : rang {trouve} (entrée supprimée, insérée ou déplacée)"
                )
            }
            Self::EmpreinteAlteree { position } => {
                write!(f, "entrée {position} ou son empreinte altérée")
            }
            Self::TeteDifferente => f.write_str("tête différente de la chaîne recalculée"),
            Self::NombreDifferent { declare, lu } => {
                write!(f, "{declare} entrée(s) déclarée(s), {lu} lue(s)")
            }
        }
    }
}

impl AnomalieAudit {
    /// Code stable de l'anomalie (réponses de l'API).
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EntreeIllisible { .. } => "entree_illisible",
            Self::PorteeDifferente { .. } => "portee_differente",
            Self::RangDifferent { .. } => "rang_different",
            Self::EmpreinteAlteree { .. } => "empreinte_alteree",
            Self::TeteDifferente => "tete_differente",
            Self::NombreDifferent { .. } => "nombre_different",
        }
    }

    /// Position de l'entrée en cause, s'il y en a une.
    #[must_use]
    pub const fn position(&self) -> Option<u64> {
        match self {
            Self::EntreeIllisible { position, .. }
            | Self::PorteeDifferente { position }
            | Self::RangDifferent { position, .. }
            | Self::EmpreinteAlteree { position } => Some(*position),
            Self::TeteDifferente | Self::NombreDifferent { .. } => None,
        }
    }
}

/// Résultat de la vérification d'une chaîne d'audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RapportAudit {
    /// La portée vérifiée.
    pub portee: Portee,
    /// Nombre d'entrées lues.
    pub nombre: u64,
    /// Tête recalculée depuis l'empreinte de départ.
    pub tete: Empreinte,
    /// Anomalies, dans l'ordre de la chaîne.
    pub anomalies: Vec<AnomalieAudit>,
}

impl RapportAudit {
    /// Vrai si aucune anomalie n'a été constatée.
    #[must_use]
    pub fn est_integre(&self) -> bool {
        self.anomalies.is_empty()
    }
}

impl fmt::Display for RapportAudit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = self.portee.identifiant();
        writeln!(f, "Audit de portée {} {id}", self.portee.nom())?;
        writeln!(
            f,
            "{} entrée(s), tête recalculée {}",
            self.nombre, self.tete
        )?;
        if self.anomalies.is_empty() {
            writeln!(f, "Résultat : chaîne d'audit intègre.")?;
        } else {
            writeln!(f, "Résultat : {} anomalie(s).", self.anomalies.len())?;
            for a in &self.anomalies {
                writeln!(f, "- {a}")?;
            }
        }
        writeln!(
            f,
            "Rappel : sans référence externe de la tête, une réécriture complète de la chaîne \
             n'est pas détectable."
        )
    }
}

/// Vérification d'une chaîne d'audit, entrée par entrée, sans tout garder en mémoire.
#[derive(Debug, Clone)]
pub struct Verificateur {
    portee: Portee,
    /// Chaîne recalculée depuis l'empreinte de départ.
    recalculee: Empreinte,
    /// Dernière empreinte déclarée : localise l'altération.
    precedente_declaree: Empreinte,
    nombre: u64,
    anomalies: Vec<AnomalieAudit>,
}

impl Verificateur {
    /// Vérification de la chaîne de `portee`.
    #[must_use]
    pub fn nouveau(portee: Portee) -> Self {
        let depart = genese(&portee);
        Self {
            portee,
            recalculee: depart,
            precedente_declaree: depart,
            nombre: 0,
            anomalies: Vec::new(),
        }
    }

    /// Entrée suivante : son contenu, l'empreinte déclarée pour elle et, en base, le rang
    /// enregistré à côté d'elle. Renvoie l'entrée si elle est lisible.
    pub fn entree(
        &mut self,
        contenu: &str,
        empreinte_declaree: &Empreinte,
        rang_enregistre: Option<u64>,
    ) -> Option<EntreeAudit> {
        self.nombre += 1;
        let position = self.nombre;
        let lue = match EntreeAudit::lire(contenu) {
            Ok(entree) => {
                if *entree.portee() != self.portee {
                    self.anomalies
                        .push(AnomalieAudit::PorteeDifferente { position });
                }
                Some(entree)
            }
            Err(erreur) => {
                self.anomalies
                    .push(AnomalieAudit::EntreeIllisible { position, erreur });
                None
            }
        };
        // Rang porté par l'entrée, sinon rang enregistré à côté d'elle : une anomalie au plus.
        let trouve = lue
            .as_ref()
            .map(EntreeAudit::rang)
            .filter(|r| *r != position)
            .or(rang_enregistre.filter(|r| *r != position));
        if let Some(trouve) = trouve {
            self.anomalies
                .push(AnomalieAudit::RangDifferent { position, trouve });
        }
        if chainer(&self.precedente_declaree, contenu.as_bytes()) != *empreinte_declaree {
            self.anomalies
                .push(AnomalieAudit::EmpreinteAlteree { position });
        }
        self.recalculee = chainer(&self.recalculee, contenu.as_bytes());
        self.precedente_declaree = *empreinte_declaree;
        lue
    }

    /// Termine la vérification : compare la tête recalculée et le nombre lu à ceux déclarés
    /// (tête enregistrée en base, ou `tete=` et `nombre=` d'un export).
    #[must_use]
    pub fn terminer(
        mut self,
        tete_declaree: Option<&Empreinte>,
        nombre_declare: Option<u64>,
    ) -> RapportAudit {
        if let Some(declare) = nombre_declare
            && declare != self.nombre
        {
            self.anomalies.push(AnomalieAudit::NombreDifferent {
                declare,
                lu: self.nombre,
            });
        }
        if let Some(tete) = tete_declaree
            && *tete != self.recalculee
        {
            self.anomalies.push(AnomalieAudit::TeteDifferente);
        }
        RapportAudit {
            portee: self.portee,
            nombre: self.nombre,
            tete: self.recalculee,
            anomalies: self.anomalies,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::entree::{Acteur, Evenement, Ressource, Symbole, Uuid};

    fn uuid(n: u8) -> Uuid {
        Uuid::depuis_octets([n; 16])
    }

    fn entree(portee: Portee, rang: u64) -> EntreeAudit {
        EntreeAudit::nouvelle(
            portee,
            rang,
            time::macros::datetime!(2026-11-19 10:00 UTC),
            Evenement::new(
                Acteur::Systeme,
                Symbole::parse("Essai").unwrap(),
                Ressource {
                    genre: Symbole::parse("cabinet").unwrap(),
                    id: portee.cabinet(),
                },
            ),
        )
        .unwrap()
    }

    fn chaine(portee: Portee, n: u64) -> (Vec<(String, Empreinte)>, Empreinte) {
        let mut c = ChaineAudit::ouvrir(portee);
        let lignes = (1..=n)
            .map(|r| c.ajouter(&entree(portee, r)).unwrap())
            .collect();
        (lignes, *c.tete())
    }

    fn verifier(portee: Portee, lignes: &[(String, Empreinte)], tete: &Empreinte) -> RapportAudit {
        let mut v = Verificateur::nouveau(portee);
        for (i, (contenu, empreinte)) in lignes.iter().enumerate() {
            v.entree(contenu, empreinte, Some(i as u64 + 1));
        }
        v.terminer(Some(tete), Some(lignes.len() as u64))
    }

    const DOSSIER: Portee = Portee::Dossier {
        cabinet: Uuid::depuis_octets([1; 16]),
        dossier: Uuid::depuis_octets([2; 16]),
    };

    #[test]
    fn genese_propre_a_la_portee_et_a_l_identifiant() {
        let cabinet = Portee::Cabinet { cabinet: uuid(2) };
        // Même UUID, autre portée : autre départ.
        assert_ne!(genese(&cabinet), genese(&DOSSIER));
        let autre = Portee::Dossier {
            cabinet: uuid(1),
            dossier: uuid(3),
        };
        assert_ne!(genese(&autre), genese(&DOSSIER));
    }

    #[test]
    fn chaine_intacte() {
        let (lignes, tete) = chaine(DOSSIER, 4);
        let r = verifier(DOSSIER, &lignes, &tete);
        assert!(r.est_integre(), "{r}");
        assert_eq!(r.nombre, 4);
        assert_eq!(r.tete, tete);
        // Chaîne vide : la tête est l'empreinte de départ.
        let r = verifier(DOSSIER, &[], &genese(&DOSSIER));
        assert!(r.est_integre());
    }

    #[test]
    fn ajout_refuse_hors_portee_ou_hors_rang() {
        let mut c = ChaineAudit::ouvrir(DOSSIER);
        assert_eq!(
            c.ajouter(&entree(DOSSIER, 2)),
            Err(ErreurAjout::Rang {
                attendu: 1,
                trouve: 2
            })
        );
        let autre = Portee::Cabinet { cabinet: uuid(1) };
        assert_eq!(
            c.ajouter(&entree(autre, 1)),
            Err(ErreurAjout::PorteeDifferente)
        );
    }

    #[test]
    fn un_octet_modifie_est_localise() {
        let (mut lignes, tete) = chaine(DOSSIER, 3);
        lignes[1].0 = lignes[1].0.replacen("Essai", "Essbi", 1);
        let r = verifier(DOSSIER, &lignes, &tete);
        assert_eq!(
            r.anomalies,
            vec![
                AnomalieAudit::EmpreinteAlteree { position: 2 },
                AnomalieAudit::TeteDifferente
            ]
        );
    }

    #[test]
    fn suppression_et_reordonnancement_detectes() {
        let (lignes, tete) = chaine(DOSSIER, 3);
        let mut supprimee = lignes.clone();
        supprimee.remove(1);
        let r = verifier(DOSSIER, &supprimee, &tete);
        assert!(r.anomalies.contains(&AnomalieAudit::RangDifferent {
            position: 2,
            trouve: 3
        }));
        assert!(r.anomalies.contains(&AnomalieAudit::TeteDifferente));
        let mut permutee = lignes.clone();
        permutee.swap(0, 1);
        assert!(!verifier(DOSSIER, &permutee, &tete).est_integre());
        // Dernière entrée retirée avec la tête : seul le nombre déclaré le révèle.
        let mut tronquee = lignes;
        tronquee.pop();
        let tete_tronquee = tronquee.last().unwrap().1;
        let mut v = Verificateur::nouveau(DOSSIER);
        for (i, (c, e)) in tronquee.iter().enumerate() {
            v.entree(c, e, Some(i as u64 + 1));
        }
        let r = v.terminer(Some(&tete_tronquee), Some(3));
        assert_eq!(
            r.anomalies,
            vec![AnomalieAudit::NombreDifferent { declare: 3, lu: 2 }]
        );
    }

    #[test]
    fn greffe_sur_une_autre_portee_ou_un_autre_tenant_detectee() {
        let (lignes, tete) = chaine(DOSSIER, 2);
        for autre in [
            // Même cabinet, autre dossier.
            Portee::Dossier {
                cabinet: uuid(1),
                dossier: uuid(9),
            },
            // Autre cabinet, même dossier.
            Portee::Dossier {
                cabinet: uuid(9),
                dossier: uuid(2),
            },
            // Portée du cabinet.
            Portee::Cabinet { cabinet: uuid(1) },
            Portee::Cabinet { cabinet: uuid(2) },
        ] {
            let r = verifier(autre, &lignes, &tete);
            assert!(
                r.anomalies
                    .contains(&AnomalieAudit::PorteeDifferente { position: 1 }),
                "{autre:?}"
            );
        }
        // Même en réécrivant chaque entrée pour l'autre portée, l'empreinte de départ diffère :
        // la chaîne recalculée ne retombe pas sur la tête de l'original.
        let autre = Portee::Dossier {
            cabinet: uuid(1),
            dossier: uuid(9),
        };
        assert_ne!(genese(&autre), genese(&DOSSIER));
        let (reecrite, tete_reecrite) = chaine(autre, 2);
        assert_ne!(tete_reecrite, tete);
        assert!(!verifier(DOSSIER, &reecrite, &tete).est_integre());
    }
}
