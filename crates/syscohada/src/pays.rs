//! Pack pays : valeurs par défaut d'un dossier selon son pays (ADR 0027).
//!
//! `data/pays.toml` est embarqué à la compilation et **validé à chaque chargement** : les dix-sept
//! États de l'OHADA, leur devise de tenue (avec son échelle décimale), leur fuseau (décalage
//! constant), leur langue, et les types d'identifiants légaux que le dossier peut porter. Chaque
//! entrée cite sa source. `compta-core` ne lit aucune table : il reçoit la devise de tenue en
//! paramètre ([`InfoDevise::tenue`]).
//!
//! Le pack ne contient **aucun paramètre fiscal** : ceux-ci relèvent de `fiscal-*` (ADR suivant).

use std::collections::BTreeSet;
use std::sync::OnceLock;

use compta_core::{Devise, DeviseTenue};
use serde::Deserialize;

/// Données embarquées : le pack pays.
const PAYS_TOML: &str = include_str!("../data/pays.toml");

/// Longueur maximale d'une langue (BCP 47, forme simplifiée).
pub const LANGUE_LONGUEUR_MAX: usize = 35;

/// Erreurs de chargement ou de validation du pack pays.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurPack {
    /// Le fichier n'est pas un TOML conforme au format attendu.
    #[error("pack pays illisible : {0}")]
    Syntaxe(String),
    /// Une entrée est incohérente (code mal formé ou en double, référence inconnue, source vide).
    #[error("pack pays incohérent : {0}")]
    Incoherent(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fichier {
    devise: Vec<BrutDevise>,
    fuseau: Vec<BrutFuseau>,
    identifiant: Vec<BrutIdentifiant>,
    /// Types d'identifiants bancaires des tiers (ADR 0031 point 3), communs à tous les pays.
    #[serde(default)]
    identifiant_bancaire: Vec<BrutIdentifiant>,
    pays: Vec<BrutPays>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrutDevise {
    code: String,
    intitule: String,
    decimales: u8,
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrutFuseau {
    nom: String,
    decalage_minutes: i16,
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrutIdentifiant {
    code: String,
    libelle: String,
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrutPays {
    code: String,
    nom: String,
    devise: String,
    fuseau: String,
    langue: String,
    source: String,
    #[serde(default)]
    identifiant_propre: Vec<BrutIdentifiant>,
}

/// Devise de tenue admise, avec son échelle décimale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoDevise {
    code: String,
    intitule: String,
    decimales: u8,
    source: String,
}

impl InfoDevise {
    /// Code ISO 4217.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Intitulé.
    #[must_use]
    pub fn intitule(&self) -> &str {
        &self.intitule
    }

    /// Décimales de la plus petite unité (ISO 4217).
    #[must_use]
    pub fn decimales(&self) -> u8 {
        self.decimales
    }

    /// Source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// La devise de tenue à passer à `compta-core` (toujours `Some` pour un pack chargé : code
    /// et échelle sont validés au chargement).
    #[must_use]
    pub fn tenue(&self) -> Option<DeviseTenue> {
        DeviseTenue::nouvelle(Devise::parse(&self.code).ok()?, self.decimales).ok()
    }
}

/// Fuseau admis : décalage constant par rapport à UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fuseau {
    nom: String,
    decalage_minutes: i16,
    source: String,
}

impl Fuseau {
    /// Nom IANA (`Africa/Dakar`).
    #[must_use]
    pub fn nom(&self) -> &str {
        &self.nom
    }

    /// Décalage par rapport à UTC, en minutes (constant toute l'année).
    #[must_use]
    pub fn decalage_minutes(&self) -> i16 {
        self.decalage_minutes
    }

    /// Source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Type d'identifiant légal (NINEA, RCCM…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeIdentifiant {
    code: String,
    libelle: String,
    source: String,
}

impl TypeIdentifiant {
    /// Code du type.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Libellé affiché.
    #[must_use]
    pub fn libelle(&self) -> &str {
        &self.libelle
    }

    /// Source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Un État de l'OHADA et les valeurs par défaut d'un dossier de ce pays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pays {
    code: String,
    nom: String,
    devise: String,
    fuseau: String,
    langue: String,
    source: String,
    identifiants: Vec<TypeIdentifiant>,
}

impl Pays {
    /// Code ISO 3166-1 alpha-2.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Nom usuel.
    #[must_use]
    pub fn nom(&self) -> &str {
        &self.nom
    }

    /// Devise de tenue par défaut (code ISO 4217).
    #[must_use]
    pub fn devise(&self) -> &str {
        &self.devise
    }

    /// Fuseau par défaut (nom IANA).
    #[must_use]
    pub fn fuseau(&self) -> &str {
        &self.fuseau
    }

    /// Langue par défaut (BCP 47).
    #[must_use]
    pub fn langue(&self) -> &str {
        &self.langue
    }

    /// Source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Types d'identifiants légaux admis pour un dossier de ce pays : les types communs, puis les
    /// types propres au pays.
    #[must_use]
    pub fn identifiants(&self) -> &[TypeIdentifiant] {
        &self.identifiants
    }

    /// Le type d'identifiant de ce code, s'il est admis pour ce pays.
    #[must_use]
    pub fn type_identifiant(&self, code: &str) -> Option<&TypeIdentifiant> {
        self.identifiants.iter().find(|t| t.code == code)
    }
}

/// Le pack pays : États, devises, fuseaux, types d'identifiants bancaires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackPays {
    pays: Vec<Pays>,
    devises: Vec<InfoDevise>,
    fuseaux: Vec<Fuseau>,
    identifiants_bancaires: Vec<TypeIdentifiant>,
}

impl PackPays {
    /// Le pack embarqué, chargé et validé une seule fois.
    ///
    /// # Errors
    /// Données embarquées invalides (jamais en pratique : le test de validation les contrôle).
    pub fn officiel() -> Result<&'static PackPays, ErreurPack> {
        static PACK: OnceLock<Result<PackPays, ErreurPack>> = OnceLock::new();
        PACK.get_or_init(Self::charger)
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Charge et valide les données embarquées.
    ///
    /// # Errors
    /// [`ErreurPack`] si une entrée est invalide.
    pub fn charger() -> Result<Self, ErreurPack> {
        Self::depuis_toml(PAYS_TOML)
    }

    /// Charge et valide un texte TOML (tests).
    ///
    /// # Errors
    /// [`ErreurPack`] si le texte est illisible ou incohérent.
    // Une seule passe de validation, table par table : la découper n'éclairerait pas.
    #[allow(clippy::too_many_lines)]
    pub fn depuis_toml(texte: &str) -> Result<Self, ErreurPack> {
        let f: Fichier = toml::from_str(texte).map_err(|e| ErreurPack::Syntaxe(e.to_string()))?;
        let incoherent = |m: String| ErreurPack::Incoherent(m);
        let source_ok = |s: &str| !s.trim().is_empty() && s.chars().count() <= 500;
        let texte_ok = |s: &str| {
            !s.trim().is_empty() && s.chars().count() <= 200 && !s.chars().any(char::is_control)
        };

        let mut devises = Vec::new();
        let mut vus = BTreeSet::new();
        for d in f.devise {
            if Devise::parse(&d.code).is_err() || !vus.insert(d.code.clone()) {
                return Err(incoherent(format!(
                    "devise {} mal formée ou en double",
                    d.code
                )));
            }
            if DeviseTenue::nouvelle(
                Devise::parse(&d.code).map_err(|e| incoherent(e.to_string()))?,
                d.decimales,
            )
            .is_err()
            {
                return Err(incoherent(format!(
                    "devise {} : échelle {} hors 0 à 4",
                    d.code, d.decimales
                )));
            }
            if !texte_ok(&d.intitule) || !source_ok(&d.source) {
                return Err(incoherent(format!(
                    "devise {} : intitulé ou source invalide",
                    d.code
                )));
            }
            devises.push(InfoDevise {
                code: d.code,
                intitule: d.intitule,
                decimales: d.decimales,
                source: d.source,
            });
        }

        let mut fuseaux = Vec::new();
        let mut vus = BTreeSet::new();
        for z in f.fuseau {
            let forme = z.nom.len() <= 64
                && z.nom.split('/').count() >= 2
                && z.nom.split('/').all(|p| {
                    !p.is_empty()
                        && p.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                });
            if !forme || !vus.insert(z.nom.clone()) {
                return Err(incoherent(format!(
                    "fuseau {} mal formé ou en double",
                    z.nom
                )));
            }
            if !(-720..=840).contains(&z.decalage_minutes) || !source_ok(&z.source) {
                return Err(incoherent(format!(
                    "fuseau {} : décalage ou source invalide",
                    z.nom
                )));
            }
            fuseaux.push(Fuseau {
                nom: z.nom,
                decalage_minutes: z.decalage_minutes,
                source: z.source,
            });
        }

        let type_ok = |t: &BrutIdentifiant| {
            let code_ok = (2..=32).contains(&t.code.len())
                && t.code.as_bytes()[0].is_ascii_uppercase()
                && t.code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            code_ok && texte_ok(&t.libelle) && source_ok(&t.source)
        };
        let converti = |t: BrutIdentifiant| TypeIdentifiant {
            code: t.code,
            libelle: t.libelle,
            source: t.source,
        };
        let mut communs = Vec::new();
        for t in f.identifiant {
            if !type_ok(&t) {
                return Err(incoherent(format!(
                    "type d'identifiant {} invalide",
                    t.code
                )));
            }
            communs.push(converti(t));
        }

        // Types d'identifiants bancaires (RIB, IBAN, portefeuille…) : même forme que les types
        // d'identifiants légaux, sans doublon. Aucune règle de forme : le pack n'en porte pas.
        let mut identifiants_bancaires: Vec<TypeIdentifiant> = Vec::new();
        for t in f.identifiant_bancaire {
            if !type_ok(&t) || identifiants_bancaires.iter().any(|x| x.code == t.code) {
                return Err(incoherent(format!(
                    "type d'identifiant bancaire {} invalide ou en double",
                    t.code
                )));
            }
            identifiants_bancaires.push(converti(t));
        }

        let mut pays = Vec::new();
        let mut vus = BTreeSet::new();
        for p in f.pays {
            let code_ok = p.code.len() == 2 && p.code.bytes().all(|b| b.is_ascii_uppercase());
            if !code_ok || !vus.insert(p.code.clone()) {
                return Err(incoherent(format!(
                    "pays {} mal formé ou en double",
                    p.code
                )));
            }
            if !devises.iter().any(|d| d.code == p.devise) {
                return Err(incoherent(format!(
                    "pays {} : devise {} inconnue",
                    p.code, p.devise
                )));
            }
            if !fuseaux.iter().any(|z| z.nom == p.fuseau) {
                return Err(incoherent(format!(
                    "pays {} : fuseau {} inconnu",
                    p.code, p.fuseau
                )));
            }
            if !langue_valide(&p.langue) || !texte_ok(&p.nom) || !source_ok(&p.source) {
                return Err(incoherent(format!(
                    "pays {} : langue, nom ou source invalide",
                    p.code
                )));
            }
            let mut identifiants = communs.clone();
            for t in p.identifiant_propre {
                if !type_ok(&t) || identifiants.iter().any(|x| x.code == t.code) {
                    return Err(incoherent(format!(
                        "pays {} : type d'identifiant {} invalide ou en double",
                        p.code, t.code
                    )));
                }
                identifiants.push(converti(t));
            }
            pays.push(Pays {
                code: p.code,
                nom: p.nom,
                devise: p.devise,
                fuseau: p.fuseau,
                langue: p.langue,
                source: p.source,
                identifiants,
            });
        }
        if pays.is_empty() {
            return Err(incoherent("aucun pays".to_owned()));
        }
        Ok(Self {
            pays,
            devises,
            fuseaux,
            identifiants_bancaires,
        })
    }

    /// Les États du pack, dans l'ordre du fichier.
    #[must_use]
    pub fn pays(&self) -> &[Pays] {
        &self.pays
    }

    /// Un État par son code ISO 3166-1 alpha-2.
    #[must_use]
    pub fn pays_par_code(&self, code: &str) -> Option<&Pays> {
        self.pays.iter().find(|p| p.code == code)
    }

    /// Les devises de tenue admises.
    #[must_use]
    pub fn devises(&self) -> &[InfoDevise] {
        &self.devises
    }

    /// Une devise de tenue admise, par son code.
    #[must_use]
    pub fn devise(&self, code: &str) -> Option<&InfoDevise> {
        self.devises.iter().find(|d| d.code == code)
    }

    /// Les fuseaux admis.
    #[must_use]
    pub fn fuseaux(&self) -> &[Fuseau] {
        &self.fuseaux
    }

    /// Un fuseau admis, par son nom IANA.
    #[must_use]
    pub fn fuseau(&self, nom: &str) -> Option<&Fuseau> {
        self.fuseaux.iter().find(|z| z.nom == nom)
    }

    /// Types d'identifiants bancaires des coordonnées d'un tiers (ADR 0031 point 3).
    #[must_use]
    pub fn identifiants_bancaires(&self) -> &[TypeIdentifiant] {
        &self.identifiants_bancaires
    }

    /// Le type d'identifiant bancaire de ce code, s'il existe.
    #[must_use]
    pub fn type_identifiant_bancaire(&self, code: &str) -> Option<&TypeIdentifiant> {
        self.identifiants_bancaires.iter().find(|t| t.code == code)
    }
}

/// Forme d'une langue BCP 47 (simplifiée) : sous-étiquettes alphanumériques de 1 à 8 caractères
/// séparées par `-`, la première de 2 ou 3 lettres minuscules (`fr`, `fr-SN`, `pt`).
#[must_use]
pub fn langue_valide(langue: &str) -> bool {
    if langue.len() > LANGUE_LONGUEUR_MAX {
        return false;
    }
    let mut parties = langue.split('-');
    let premiere = parties.next().unwrap_or("");
    let ok_premiere =
        (2..=3).contains(&premiere.len()) && premiere.bytes().all(|b| b.is_ascii_lowercase());
    ok_premiere
        && parties
            .all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_donnees_embarquees_sont_valides() {
        let pack = PackPays::charger().unwrap();
        assert_eq!(PackPays::officiel().unwrap(), &pack);
    }

    #[test]
    fn types_d_identifiants_bancaires() {
        // ADR 0031 point 3 : RIB, IBAN, portefeuille, autre ; aucune règle de forme.
        let pack = PackPays::officiel().unwrap();
        let codes: Vec<&str> = pack
            .identifiants_bancaires()
            .iter()
            .map(TypeIdentifiant::code)
            .collect();
        assert_eq!(codes, ["RIB", "IBAN", "PORTEFEUILLE", "AUTRE"]);
        assert!(pack.type_identifiant_bancaire("NINEA").is_none());
        let double = PAYS_TOML.replace(
            "[[identifiant_bancaire]]\ncode = \"IBAN\"",
            "[[identifiant_bancaire]]\ncode = \"RIB\"",
        );
        assert!(matches!(
            PackPays::depuis_toml(&double),
            Err(ErreurPack::Incoherent(_))
        ));
    }

    #[test]
    fn dix_sept_etats_de_l_ohada() {
        // Guide d'application, p. PDF 394, note (2).
        let pack = PackPays::officiel().unwrap();
        let codes: BTreeSet<&str> = pack.pays().iter().map(Pays::code).collect();
        let attendus: BTreeSet<&str> = [
            "BJ", "BF", "CI", "GW", "ML", "NE", "SN", "TG", "CM", "CG", "GA", "CF", "TD", "KM",
            "GN", "GQ", "CD",
        ]
        .into_iter()
        .collect();
        assert_eq!(codes, attendus);
    }

    #[test]
    fn devise_par_pays_et_echelle() {
        let pack = PackPays::officiel().unwrap();
        let attendu = |pays: &str, devise: &str, decimales: u8| {
            let p = pack.pays_par_code(pays).unwrap();
            assert_eq!(p.devise(), devise, "{pays}");
            assert_eq!(
                pack.devise(devise).unwrap().decimales(),
                decimales,
                "{devise}"
            );
        };
        for p in ["BJ", "BF", "CI", "GW", "ML", "NE", "SN", "TG"] {
            attendu(p, "XOF", 0);
        }
        for p in ["CM", "CG", "GA", "CF", "TD", "GQ"] {
            attendu(p, "XAF", 0);
        }
        attendu("KM", "KMF", 0);
        attendu("GN", "GNF", 0);
        attendu("CD", "CDF", 2);
        assert_eq!(pack.devise("EUR").unwrap().decimales(), 2);
        assert_eq!(pack.devise("USD").unwrap().decimales(), 2);
        assert!(pack.devise("ZZZ").is_none());
    }

    #[test]
    fn la_tenue_se_construit_pour_chaque_devise() {
        let pack = PackPays::officiel().unwrap();
        for d in pack.devises() {
            let t = d.tenue().unwrap();
            assert_eq!(t.devise().code(), d.code());
            assert_eq!(t.decimales(), d.decimales());
        }
    }

    #[test]
    fn fuseaux_sans_heure_d_ete() {
        let pack = PackPays::officiel().unwrap();
        assert_eq!(pack.fuseau("Africa/Dakar").unwrap().decalage_minutes(), 0);
        assert_eq!(pack.fuseau("Africa/Douala").unwrap().decalage_minutes(), 60);
        assert_eq!(
            pack.fuseau("Africa/Lubumbashi").unwrap().decalage_minutes(),
            120
        );
        assert_eq!(
            pack.fuseau("Indian/Comoro").unwrap().decalage_minutes(),
            180
        );
        assert!(pack.fuseau("Europe/Paris").is_none());
        assert!(pack.fuseau("africa/dakar").is_none());
    }

    #[test]
    fn identifiants_legaux_par_pays() {
        let pack = PackPays::officiel().unwrap();
        let sn = pack.pays_par_code("SN").unwrap();
        let codes: Vec<&str> = sn
            .identifiants()
            .iter()
            .map(TypeIdentifiant::code)
            .collect();
        assert_eq!(codes, ["RCCM", "AUTRE", "NINEA"]);
        assert!(sn.type_identifiant("NINEA").is_some());
        // NINEA est propre au Sénégal : aucune colonne, aucun type chez les autres.
        let cm = pack.pays_par_code("CM").unwrap();
        assert!(cm.type_identifiant("NINEA").is_none());
        assert!(cm.type_identifiant("RCCM").is_some());
    }

    #[test]
    fn chaque_entree_cite_sa_source() {
        let pack = PackPays::officiel().unwrap();
        assert!(pack.pays().iter().all(|p| !p.source().is_empty()));
        assert!(pack.devises().iter().all(|d| !d.source().is_empty()));
        assert!(pack.fuseaux().iter().all(|z| !z.source().is_empty()));
        assert!(
            pack.pays()
                .iter()
                .flat_map(Pays::identifiants)
                .all(|t| !t.source().is_empty())
        );
    }

    #[test]
    fn langues_bcp47() {
        for ok in ["fr", "fr-SN", "pt", "es-GQ", "fr-Latn-SN"] {
            assert!(langue_valide(ok), "{ok}");
        }
        for ko in [
            "",
            "f",
            "FR",
            "fr_SN",
            "fr-",
            "-fr",
            "fr--SN",
            "français",
            "fr-é",
            &"a".repeat(36),
        ] {
            assert!(!langue_valide(ko), "{ko}");
        }
    }

    #[test]
    fn refus_des_donnees_incoherentes() {
        let base = include_str!("../data/pays.toml");
        for (avant, apres) in [
            (
                "devise = \"XAF\"\nfuseau = \"Africa/Douala\"",
                "devise = \"YYY\"\nfuseau = \"Africa/Douala\"",
            ),
            ("fuseau = \"Africa/Dakar\"", "fuseau = \"Europe/Paris\""),
            ("code = \"SN\"\nnom", "code = \"BJ\"\nnom"),
            ("decimales = 2", "decimales = 9"),
            ("langue = \"fr\"", "langue = \"FR\""),
            (
                "[[pays.identifiant_propre]]\ncode = \"NINEA\"",
                "[[pays.identifiant_propre]]\ncode = \"RCCM\"",
            ),
        ] {
            let texte = base.replacen(avant, apres, 1);
            assert_ne!(texte, base, "{avant}");
            assert!(PackPays::depuis_toml(&texte).is_err(), "{apres}");
        }
        assert!(PackPays::depuis_toml("n'importe quoi").is_err());
    }
}
