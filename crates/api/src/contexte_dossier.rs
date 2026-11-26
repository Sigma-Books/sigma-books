//! Contexte d'un dossier (ADR 0027) : pays, devise de tenue et échelle, fuseau, langue, forme
//! juridique, identifiants légaux. Pur : aucune base, aucune horloge.
//!
//! Les valeurs par défaut viennent du **pack pays** (`syscohada`, `data/pays.toml`) : le pays est
//! obligatoire, la devise de tenue, le fuseau et la langue valent par défaut ceux du pays. Une
//! valeur fournie est contrôlée contre les tables du pack (devises, fuseaux, types d'identifiants
//! du pays). Le contexte est **fixé à la création** et ne change plus (la devise de tenue est
//! dans le contenu haché de chaque écriture).
//!
//! Le serveur ne lit pas la base de fuseaux de l'IANA : le pack donne le décalage constant de
//! chaque fuseau admis (aucun n'observe l'heure d'été).

use compta_core::{Devise, DeviseTenue};
use syscohada::{PackPays, langue_valide};
use time::{Date, OffsetDateTime, UtcOffset};

use crate::base::ErreurBase;

/// Nombre maximal d'identifiants légaux d'un dossier.
pub const IDENTIFIANTS_MAX: usize = 10;
/// Longueur maximale (en caractères) de la valeur d'un identifiant légal.
pub const VALEUR_IDENTIFIANT_MAX: usize = 64;
/// Longueur maximale (en caractères) de la forme juridique.
pub const FORME_JURIDIQUE_MAX: usize = 100;

/// Contexte d'un dossier, tel qu'il est stocké et rendu à l'interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContexteDossier {
    /// Pays (ISO 3166-1 alpha-2).
    pub pays: String,
    /// Devise de tenue (ISO 4217).
    pub devise: String,
    /// Décimales de la devise de tenue (copiées du pack à la création).
    pub decimales: u8,
    /// Fuseau (nom IANA) dans lequel se calcule la date du jour.
    pub fuseau: String,
    /// Langue (BCP 47).
    pub langue: String,
    /// Forme juridique (texte libre), si elle est renseignée.
    pub forme_juridique: Option<String>,
}

/// Un identifiant légal : un type du pack pays (`NINEA`, `RCCM`…) et sa valeur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiantLegal {
    /// Code du type, tel que le pack du pays le définit.
    pub type_identifiant: String,
    /// Valeur, texte libre borné.
    pub valeur: String,
}

/// Demande de contexte à la création d'un dossier.
#[derive(Debug, Clone, Copy)]
pub struct DemandeContexte<'a> {
    /// Pays : obligatoire.
    pub pays: &'a str,
    /// Devise de tenue, par défaut celle du pays.
    pub devise: Option<&'a str>,
    /// Fuseau, par défaut celui du pays.
    pub fuseau: Option<&'a str>,
    /// Langue, par défaut celle du pays.
    pub langue: Option<&'a str>,
    /// Forme juridique.
    pub forme_juridique: Option<&'a str>,
}

/// Motif de refus d'un contexte demandé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusContexte {
    /// Pays absent du pack (les dix-sept États de l'OHADA).
    PaysInconnu,
    /// Devise de tenue absente de la table des devises du pack.
    DeviseInconnue,
    /// Fuseau absent de la table des fuseaux du pack.
    FuseauInconnu,
    /// Langue, forme juridique ou identifiant légal invalide (forme, longueur, type du pays,
    /// doublon, nombre).
    Invalide,
}

/// Longueur en caractères, sans caractère de contrôle, non vide après retrait des blancs.
fn texte_borne(texte: &str, max: usize) -> bool {
    !texte.trim().is_empty() && texte.chars().count() <= max && !texte.chars().any(char::is_control)
}

/// Résout le contexte d'un dossier à créer : valeurs par défaut du pays, puis contrôle de
/// chaque valeur fournie.
///
/// # Errors
/// [`RefusContexte`] : pays, devise, fuseau inconnus du pack ; langue ou forme juridique invalide.
pub fn resoudre(
    pack: &PackPays,
    demande: &DemandeContexte<'_>,
) -> Result<ContexteDossier, RefusContexte> {
    let pays = pack
        .pays_par_code(demande.pays)
        .ok_or(RefusContexte::PaysInconnu)?;
    let devise = demande.devise.unwrap_or_else(|| pays.devise());
    let info = pack.devise(devise).ok_or(RefusContexte::DeviseInconnue)?;
    let fuseau = demande.fuseau.unwrap_or_else(|| pays.fuseau());
    let fuseau = pack.fuseau(fuseau).ok_or(RefusContexte::FuseauInconnu)?;
    let langue = demande.langue.unwrap_or_else(|| pays.langue());
    if !langue_valide(langue) {
        return Err(RefusContexte::Invalide);
    }
    if let Some(forme) = demande.forme_juridique
        && !texte_borne(forme, FORME_JURIDIQUE_MAX)
    {
        return Err(RefusContexte::Invalide);
    }
    Ok(ContexteDossier {
        pays: pays.code().to_owned(),
        devise: info.code().to_owned(),
        decimales: info.decimales(),
        fuseau: fuseau.nom().to_owned(),
        langue: langue.to_owned(),
        forme_juridique: demande.forme_juridique.map(str::to_owned),
    })
}

/// Modification demandée du contexte d'un dossier existant : chaque champ absent est conservé.
/// `forme_juridique` : `Some(None)` l'efface.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModificationContexte<'a> {
    /// Nouveau pays.
    pub pays: Option<&'a str>,
    /// Nouvelle devise de tenue (l'échelle suit, du pack).
    pub devise: Option<&'a str>,
    /// Nouveau fuseau.
    pub fuseau: Option<&'a str>,
    /// Nouvelle langue.
    pub langue: Option<&'a str>,
    /// Nouvelle forme juridique, ou son effacement.
    pub forme_juridique: Option<Option<&'a str>>,
}

impl ModificationContexte<'_> {
    /// Vrai si la demande ne porte sur aucun champ.
    #[must_use]
    pub fn est_vide(&self) -> bool {
        self.pays.is_none()
            && self.devise.is_none()
            && self.fuseau.is_none()
            && self.langue.is_none()
            && self.forme_juridique.is_none()
    }
}

/// Applique une modification au contexte actuel : mêmes contrôles qu'à la création, contre les
/// tables du pack. Une devise identique à l'actuelle conserve l'échelle stockée ; une devise
/// différente prend l'échelle du pack.
///
/// # Errors
/// [`RefusContexte`] : pays, devise, fuseau inconnus du pack ; langue ou forme juridique invalide.
pub fn modifier(
    pack: &PackPays,
    actuel: &ContexteDossier,
    modification: &ModificationContexte<'_>,
) -> Result<ContexteDossier, RefusContexte> {
    let pays = modification.pays.unwrap_or(&actuel.pays);
    pack.pays_par_code(pays).ok_or(RefusContexte::PaysInconnu)?;
    let (devise, decimales) = match modification.devise {
        Some(d) if d != actuel.devise => {
            let info = pack.devise(d).ok_or(RefusContexte::DeviseInconnue)?;
            (info.code().to_owned(), info.decimales())
        }
        _ => (actuel.devise.clone(), actuel.decimales),
    };
    let fuseau = modification.fuseau.unwrap_or(&actuel.fuseau);
    pack.fuseau(fuseau).ok_or(RefusContexte::FuseauInconnu)?;
    let langue = modification.langue.unwrap_or(&actuel.langue);
    if !langue_valide(langue) {
        return Err(RefusContexte::Invalide);
    }
    let forme_juridique = match modification.forme_juridique {
        None => actuel.forme_juridique.clone(),
        Some(None) => None,
        Some(Some(f)) if texte_borne(f, FORME_JURIDIQUE_MAX) => Some(f.to_owned()),
        Some(Some(_)) => return Err(RefusContexte::Invalide),
    };
    Ok(ContexteDossier {
        pays: pays.to_owned(),
        devise,
        decimales,
        fuseau: fuseau.to_owned(),
        langue: langue.to_owned(),
        forme_juridique,
    })
}

/// Contrôle les identifiants légaux demandés pour un pays : au plus [`IDENTIFIANTS_MAX`], un seul
/// par type, type admis pour le pays, valeur de 1 à [`VALEUR_IDENTIFIANT_MAX`] caractères sans
/// caractère de contrôle. Aucun format national n'est imposé (aucun n'est sourcé).
///
/// # Errors
/// [`RefusContexte::PaysInconnu`] ou [`RefusContexte::Invalide`].
pub fn controler_identifiants(
    pack: &PackPays,
    pays: &str,
    demandes: &[IdentifiantLegal],
) -> Result<(), RefusContexte> {
    let pays = pack.pays_par_code(pays).ok_or(RefusContexte::PaysInconnu)?;
    if demandes.len() > IDENTIFIANTS_MAX {
        return Err(RefusContexte::Invalide);
    }
    for (i, d) in demandes.iter().enumerate() {
        let doublon = demandes[..i]
            .iter()
            .any(|p| p.type_identifiant == d.type_identifiant);
        if doublon
            || pays.type_identifiant(&d.type_identifiant).is_none()
            || !texte_borne(&d.valeur, VALEUR_IDENTIFIANT_MAX)
        {
            return Err(RefusContexte::Invalide);
        }
    }
    Ok(())
}

impl ContexteDossier {
    /// Contexte lu en base : contrôle que les valeurs relues sont celles d'un contexte valide
    /// (jamais produit autrement que par la création).
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si une valeur n'a pas la forme attendue.
    pub fn relu(
        pays: String,
        devise: String,
        decimales: i16,
        fuseau: String,
        langue: String,
        forme_juridique: Option<String>,
    ) -> Result<Self, ErreurBase> {
        let decimales = u8::try_from(decimales)
            .ok()
            .filter(|d| *d <= DeviseTenue::DECIMALES_MAX)
            .ok_or(ErreurBase::Donnee("échelle de la devise de tenue invalide"))?;
        Ok(Self {
            pays,
            devise,
            decimales,
            fuseau,
            langue,
            forme_juridique,
        })
    }

    /// Vrai si le pays, la devise de tenue ou son échelle diffèrent : ces champs entrent dans le
    /// contenu haché (devise) et ne changent plus après la première écriture (ADR 0027).
    #[must_use]
    pub fn change_pays_ou_devise(&self, autre: &Self) -> bool {
        self.pays != autre.pays || self.devise != autre.devise || self.decimales != autre.decimales
    }

    /// Devise de tenue et échelle, à passer à `compta-core`.
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si le code de devise stocké est invalide.
    pub fn tenue(&self) -> Result<DeviseTenue, ErreurBase> {
        let devise = Devise::parse(&self.devise)
            .map_err(|_| ErreurBase::Donnee("devise de tenue invalide"))?;
        DeviseTenue::nouvelle(devise, self.decimales)
            .map_err(|_| ErreurBase::Donnee("échelle de la devise de tenue invalide"))
    }

    /// Décalage du fuseau du dossier par rapport à UTC.
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si le fuseau stocké n'est pas dans le pack.
    pub fn decalage(&self) -> Result<UtcOffset, ErreurBase> {
        let pack = PackPays::officiel().map_err(|_| ErreurBase::Donnee("pack pays invalide"))?;
        let fuseau = pack
            .fuseau(&self.fuseau)
            .ok_or(ErreurBase::Donnee("fuseau du dossier inconnu"))?;
        UtcOffset::from_whole_seconds(i32::from(fuseau.decalage_minutes()) * 60)
            .map_err(|_| ErreurBase::Donnee("décalage du fuseau invalide"))
    }

    /// Date du jour **dans le fuseau du dossier** (jamais celle d'UTC) pour un instant donné.
    ///
    /// # Errors
    /// [`ErreurBase::Donnee`] si le fuseau stocké est inconnu.
    pub fn date_du_jour(&self, instant: OffsetDateTime) -> Result<Date, ErreurBase> {
        Ok(instant.to_offset(self.decalage()?).date())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn pack() -> &'static PackPays {
        PackPays::officiel().unwrap()
    }

    fn demande(pays: &str) -> DemandeContexte<'_> {
        DemandeContexte {
            pays,
            devise: None,
            fuseau: None,
            langue: None,
            forme_juridique: None,
        }
    }

    #[test]
    fn valeurs_par_defaut_du_pays() {
        let c = resoudre(pack(), &demande("SN")).unwrap();
        assert_eq!(
            (
                c.devise.as_str(),
                c.decimales,
                c.fuseau.as_str(),
                c.langue.as_str()
            ),
            ("XOF", 0, "Africa/Dakar", "fr")
        );
        let c = resoudre(pack(), &demande("CM")).unwrap();
        assert_eq!((c.devise.as_str(), c.decimales), ("XAF", 0));
        let c = resoudre(pack(), &demande("CD")).unwrap();
        assert_eq!((c.devise.as_str(), c.decimales), ("CDF", 2));
        assert_eq!(c.fuseau, "Africa/Kinshasa");
        let c = resoudre(pack(), &demande("KM")).unwrap();
        assert_eq!(
            (c.devise.as_str(), c.fuseau.as_str()),
            ("KMF", "Indian/Comoro")
        );
    }

    #[test]
    fn valeurs_fournies_controlees_contre_les_tables() {
        let mut d = demande("SN");
        d.devise = Some("EUR");
        d.fuseau = Some("Africa/Lubumbashi");
        d.langue = Some("fr-SN");
        d.forme_juridique = Some("SARL");
        let c = resoudre(pack(), &d).unwrap();
        assert_eq!((c.devise.as_str(), c.decimales), ("EUR", 2));
        assert_eq!(c.fuseau, "Africa/Lubumbashi");
        assert_eq!(c.forme_juridique.as_deref(), Some("SARL"));

        assert_eq!(
            resoudre(pack(), &demande("FR")),
            Err(RefusContexte::PaysInconnu)
        );
        assert_eq!(
            resoudre(pack(), &demande("sn")),
            Err(RefusContexte::PaysInconnu)
        );
        assert_eq!(
            resoudre(pack(), &demande("")),
            Err(RefusContexte::PaysInconnu)
        );
        let mut d = demande("SN");
        d.devise = Some("XXX");
        assert_eq!(resoudre(pack(), &d), Err(RefusContexte::DeviseInconnue));
        let mut d = demande("SN");
        d.fuseau = Some("Europe/Paris");
        assert_eq!(resoudre(pack(), &d), Err(RefusContexte::FuseauInconnu));
        for ko in ["FR", "fr_SN", "", "français"] {
            let mut d = demande("SN");
            d.langue = Some(ko);
            assert_eq!(resoudre(pack(), &d), Err(RefusContexte::Invalide), "{ko}");
        }
        for ko in ["", "   ", "a\u{7}b", &"é".repeat(101)] {
            let mut d = demande("SN");
            d.forme_juridique = Some(ko);
            assert_eq!(resoudre(pack(), &d), Err(RefusContexte::Invalide), "{ko:?}");
        }
        let longue = "é".repeat(100);
        let mut d = demande("SN");
        d.forme_juridique = Some(&longue);
        assert!(resoudre(pack(), &d).is_ok());
    }

    fn id(t: &str, v: &str) -> IdentifiantLegal {
        IdentifiantLegal {
            type_identifiant: t.to_owned(),
            valeur: v.to_owned(),
        }
    }

    #[test]
    fn identifiants_du_pack_du_pays() {
        assert!(
            controler_identifiants(
                pack(),
                "SN",
                &[id("NINEA", "0012345 2G2"), id("RCCM", "SN-DKR-2020-B-1234")]
            )
            .is_ok()
        );
        assert!(controler_identifiants(pack(), "SN", &[]).is_ok());
        // NINEA est propre au Sénégal.
        assert_eq!(
            controler_identifiants(pack(), "CM", &[id("NINEA", "1")]),
            Err(RefusContexte::Invalide)
        );
        for ko in [
            vec![id("NINEA", "1"), id("NINEA", "2")],
            vec![id("INCONNU", "1")],
            vec![id("ninea", "1")],
            vec![id("NINEA", "")],
            vec![id("NINEA", "  ")],
            vec![id("NINEA", "a\nb")],
            vec![id("NINEA", &"1".repeat(65))],
        ] {
            assert_eq!(
                controler_identifiants(pack(), "SN", &ko),
                Err(RefusContexte::Invalide),
                "{ko:?}"
            );
        }
        assert!(controler_identifiants(pack(), "SN", &[id("NINEA", &"1".repeat(64))]).is_ok());
        assert_eq!(
            controler_identifiants(pack(), "ZZ", &[]),
            Err(RefusContexte::PaysInconnu)
        );
        // Au plus dix identifiants.
        let trop: Vec<_> = (0..11).map(|i| id("AUTRE", &i.to_string())).collect();
        assert_eq!(
            controler_identifiants(pack(), "SN", &trop),
            Err(RefusContexte::Invalide)
        );
    }

    #[test]
    fn tenue_pour_compta_core() {
        let c = resoudre(pack(), &demande("CD")).unwrap();
        let t = c.tenue().unwrap();
        assert_eq!((t.devise().code(), t.decimales()), ("CDF", 2));
        let mut ko = c.clone();
        ko.devise = "cdf".to_owned();
        assert!(ko.tenue().is_err());
        assert!(
            ContexteDossier::relu(
                "SN".into(),
                "XOF".into(),
                9,
                "Africa/Dakar".into(),
                "fr".into(),
                None
            )
            .is_err()
        );
        assert!(
            ContexteDossier::relu(
                "SN".into(),
                "XOF".into(),
                -1,
                "Africa/Dakar".into(),
                "fr".into(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn date_du_jour_dans_le_fuseau_du_dossier() {
        // 23 h 30 UTC le 31 décembre : déjà le 1er janvier à Douala (UTC+1) et à Moroni (UTC+3),
        // encore le 31 à Dakar (UTC+0).
        let instant = datetime!(2026-12-31 23:30 UTC);
        let jour = |pays: &str| {
            resoudre(pack(), &demande(pays))
                .unwrap()
                .date_du_jour(instant)
                .unwrap()
        };
        assert_eq!(jour("SN"), time::macros::date!(2026 - 12 - 31));
        assert_eq!(jour("CM"), time::macros::date!(2027 - 01 - 01));
        assert_eq!(jour("KM"), time::macros::date!(2027 - 01 - 01));
        // Instant donné avec un autre décalage : la date dépend du seul fuseau du dossier.
        let instant = datetime!(2026-12-31 23:30 -05:00);
        let dakar = resoudre(pack(), &demande("SN")).unwrap();
        assert_eq!(
            dakar.date_du_jour(instant).unwrap(),
            time::macros::date!(2027 - 01 - 01)
        );
        let mut inconnu = dakar;
        inconnu.fuseau = "Mars/Olympus".to_owned();
        assert!(inconnu.date_du_jour(instant).is_err());
    }

    #[test]
    fn modification_du_contexte() {
        let sn = resoudre(pack(), &demande("SN")).unwrap();
        // Une modification vide ne change rien.
        let m = ModificationContexte::default();
        assert!(m.est_vide());
        assert_eq!(modifier(pack(), &sn, &m).unwrap(), sn);
        // Devise : l'échelle suit le pack ; fuseau, langue, forme juridique changent à part.
        let m = ModificationContexte {
            pays: Some("CD"),
            devise: Some("CDF"),
            fuseau: Some("Africa/Lubumbashi"),
            langue: Some("fr-CD"),
            forme_juridique: Some(Some("SA")),
        };
        assert!(!m.est_vide());
        let cd = modifier(pack(), &sn, &m).unwrap();
        assert_eq!(
            (cd.pays.as_str(), cd.devise.as_str(), cd.decimales),
            ("CD", "CDF", 2)
        );
        assert_eq!(cd.fuseau, "Africa/Lubumbashi");
        assert!(sn.change_pays_ou_devise(&cd));
        // Fuseau et langue seuls : ni pays ni devise.
        let m = ModificationContexte {
            fuseau: Some("Africa/Douala"),
            ..ModificationContexte::default()
        };
        let douala = modifier(pack(), &cd, &m).unwrap();
        assert!(!cd.change_pays_ou_devise(&douala));
        // Même devise redemandée : l'échelle stockée est conservée.
        let mut stocke = cd.clone();
        stocke.decimales = 3;
        let m = ModificationContexte {
            devise: Some("CDF"),
            ..ModificationContexte::default()
        };
        assert_eq!(modifier(pack(), &stocke, &m).unwrap().decimales, 3);
        // Effacement de la forme juridique.
        let m = ModificationContexte {
            forme_juridique: Some(None),
            ..ModificationContexte::default()
        };
        assert_eq!(modifier(pack(), &cd, &m).unwrap().forme_juridique, None);
        // Refus : mêmes codes qu'à la création.
        for (m, attendu) in [
            (
                ModificationContexte {
                    pays: Some("FR"),
                    ..Default::default()
                },
                RefusContexte::PaysInconnu,
            ),
            (
                ModificationContexte {
                    devise: Some("ZZZ"),
                    ..Default::default()
                },
                RefusContexte::DeviseInconnue,
            ),
            (
                ModificationContexte {
                    fuseau: Some("Europe/Paris"),
                    ..Default::default()
                },
                RefusContexte::FuseauInconnu,
            ),
            (
                ModificationContexte {
                    langue: Some("FR"),
                    ..Default::default()
                },
                RefusContexte::Invalide,
            ),
            (
                ModificationContexte {
                    forme_juridique: Some(Some(" ")),
                    ..Default::default()
                },
                RefusContexte::Invalide,
            ),
        ] {
            assert_eq!(modifier(pack(), &sn, &m), Err(attendu));
        }
    }
}
