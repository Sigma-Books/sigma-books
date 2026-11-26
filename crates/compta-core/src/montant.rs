//! Montants en décimal exact, devise attachée (Fondations §2.4, spec §5.5 et §6.6).
//!
//! Aucun flottant : la valeur est un [`Decimal`]. Les calculs internes sont exacts ; seule
//! la contre-valeur en devise de tenue est arrondie à l'échelle de cette devise ([`DeviseTenue`] :
//! zéro décimale pour le XOF, deux pour le CDF), au plus près, la moitié s'éloignant de zéro
//! (`0,5` → `1`, `-0,5` → `-1` : pas d'arrondi bancaire). La devise de tenue et son échelle sont
//! des **paramètres** : le moteur ne suppose aucune devise (ADR 0027). Toute opération qui
//! pourrait dépasser la capacité du décimal renvoie une erreur au lieu de paniquer.

use std::fmt;
use std::str::FromStr;

use rust_decimal::{Decimal, RoundingStrategy};

/// Nombre maximal de chiffres (partie entière et décimales) accepté à l'analyse.
const CHIFFRES_MAX: usize = 28;

/// Erreurs sur les devises et les montants.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurMontant {
    /// Le code de devise n'est pas formé de trois lettres majuscules (ISO 4217).
    #[error("code de devise invalide : « {0} » (trois lettres majuscules attendues)")]
    DeviseInvalide(String),
    /// Le texte n'est pas un montant décimal simple (`-123.45`).
    #[error("montant invalide : « {0} »")]
    FormatInvalide(String),
    /// Opération entre deux devises différentes.
    #[error("devises différentes : {0} et {1}")]
    DevisesDifferentes(Devise, Devise),
    /// Le résultat dépasse la capacité du décimal.
    #[error("dépassement de capacité")]
    Depassement,
    /// Arrondi à l'échelle de la devise de tenue demandé sur un montant dans une autre devise.
    #[error("arrondi impossible : le montant est en {0}, pas dans la devise de tenue")]
    PasEnDeviseTenue(Devise),
    /// Échelle décimale d'une devise de tenue hors des bornes (0 à 4).
    #[error("échelle décimale invalide : {0} (0 à 4 attendu)")]
    EchelleInvalide(u8),
    /// Montant qui a plus de décimales que l'échelle de la devise de tenue.
    #[error("montant invalide : plus de {decimales} décimale(s) pour la devise de tenue")]
    EchelleExcessive {
        /// Échelle de la devise de tenue.
        decimales: u8,
    },
    /// Un cours de change doit être strictement positif.
    #[error("cours de change invalide : {0} (strictement positif attendu)")]
    CoursInvalide(Decimal),
}

/// Code de devise ISO 4217 (trois lettres majuscules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Devise([u8; 3]);

impl Devise {
    /// Analyse un code de devise.
    ///
    /// # Errors
    /// [`ErreurMontant::DeviseInvalide`] si le code n'est pas formé de trois lettres majuscules ASCII.
    pub fn parse(code: &str) -> Result<Self, ErreurMontant> {
        match code.as_bytes() {
            &[a, b, c] if [a, b, c].iter().all(u8::is_ascii_uppercase) => Ok(Devise([a, b, c])),
            _ => Err(ErreurMontant::DeviseInvalide(code.to_owned())),
        }
    }

    /// Le code à trois lettres.
    #[must_use]
    pub fn code(&self) -> &str {
        // Les trois octets sont des lettres ASCII (vérifié à la construction).
        std::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Display for Devise {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Devise de tenue d'un dossier et échelle décimale de ses montants (ADR 0027).
///
/// Passée en paramètre au moteur, qui ne lit aucune table et n'en suppose aucune : l'appelant
/// la tire du dossier (pays, devise, décimales). Les décimales sont celles de la plus petite
/// unité de la devise (ISO 4217 : XOF, XAF, KMF, GNF à 0 ; CDF, EUR, USD à 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviseTenue {
    devise: Devise,
    decimales: u8,
}

impl DeviseTenue {
    /// Plus grand nombre de décimales admis (ISO 4217 : 4 au plus).
    pub const DECIMALES_MAX: u8 = 4;

    /// Construit une devise de tenue.
    ///
    /// # Errors
    /// [`ErreurMontant::EchelleInvalide`] si `decimales` dépasse [`Self::DECIMALES_MAX`].
    pub fn nouvelle(devise: Devise, decimales: u8) -> Result<Self, ErreurMontant> {
        if decimales > Self::DECIMALES_MAX {
            return Err(ErreurMontant::EchelleInvalide(decimales));
        }
        Ok(Self { devise, decimales })
    }

    /// La devise.
    #[must_use]
    pub fn devise(&self) -> Devise {
        self.devise
    }

    /// Nombre de décimales de la plus petite unité.
    #[must_use]
    pub fn decimales(&self) -> u8 {
        self.decimales
    }

    /// Arrondit une valeur à l'échelle de la devise : au plus près, la moitié s'éloignant de zéro.
    #[must_use]
    pub fn arrondir(&self, valeur: Decimal) -> Decimal {
        valeur.round_dp_with_strategy(
            u32::from(self.decimales),
            RoundingStrategy::MidpointAwayFromZero,
        )
    }
}

/// Montant exact dans une devise. Peut être négatif (un solde, une différence) ; la positivité
/// des lignes d'écriture est contrôlée à la validation (spec §6.2, invariant 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Montant {
    valeur: Decimal,
    devise: Devise,
}

impl Montant {
    /// Construit un montant à partir d'une valeur décimale exacte.
    #[must_use]
    pub fn nouveau(valeur: Decimal, devise: Devise) -> Self {
        Self { valeur, devise }
    }

    /// Montant nul dans une devise.
    #[must_use]
    pub fn zero(devise: Devise) -> Self {
        Self::nouveau(Decimal::ZERO, devise)
    }

    /// Analyse un montant écrit `-?chiffres(.chiffres)?` : pas d'exposant, pas de séparateur de
    /// milliers, pas de signe `+`, au plus 28 chiffres. L'échelle écrite est conservée.
    ///
    /// # Errors
    /// [`ErreurMontant::FormatInvalide`] si le texte ne respecte pas ce format.
    pub fn parse(texte: &str, devise: Devise) -> Result<Self, ErreurMontant> {
        Ok(Self::nouveau(analyser_decimal(texte)?, devise))
    }

    /// Analyse un montant **dans la devise de tenue** et refuse plus de décimales que son échelle
    /// (zéros finaux écrits non comptés : `100.00` vaut `100`). C'est le contrôle de saisie ;
    /// la relecture d'un montant stocké utilise [`Self::parse`], sans contrôle d'échelle.
    ///
    /// # Errors
    /// [`ErreurMontant::FormatInvalide`] ; [`ErreurMontant::EchelleExcessive`] au-delà de l'échelle.
    pub fn parse_tenue(texte: &str, tenue: &DeviseTenue) -> Result<Self, ErreurMontant> {
        let valeur = analyser_decimal(texte)?;
        if valeur.normalize().scale() > u32::from(tenue.decimales) {
            return Err(ErreurMontant::EchelleExcessive {
                decimales: tenue.decimales,
            });
        }
        Ok(Self::nouveau(valeur, tenue.devise))
    }

    /// Valeur décimale exacte.
    #[must_use]
    pub fn valeur(&self) -> Decimal {
        self.valeur
    }

    /// Devise du montant.
    #[must_use]
    pub fn devise(&self) -> Devise {
        self.devise
    }

    /// Vrai si le montant vaut zéro.
    #[must_use]
    pub fn est_nul(&self) -> bool {
        self.valeur.is_zero()
    }

    /// Vrai si le montant est strictement positif.
    #[must_use]
    pub fn est_strictement_positif(&self) -> bool {
        self.valeur > Decimal::ZERO
    }

    /// Somme exacte de deux montants de même devise.
    ///
    /// # Errors
    /// Devises différentes ou dépassement de capacité.
    pub fn ajouter(&self, autre: &Montant) -> Result<Montant, ErreurMontant> {
        self.meme_devise(autre)?;
        let valeur = self
            .valeur
            .checked_add(autre.valeur)
            .ok_or(ErreurMontant::Depassement)?;
        Ok(Self::nouveau(valeur, self.devise))
    }

    /// Différence exacte de deux montants de même devise.
    ///
    /// # Errors
    /// Devises différentes ou dépassement de capacité.
    pub fn soustraire(&self, autre: &Montant) -> Result<Montant, ErreurMontant> {
        self.meme_devise(autre)?;
        let valeur = self
            .valeur
            .checked_sub(autre.valeur)
            .ok_or(ErreurMontant::Depassement)?;
        Ok(Self::nouveau(valeur, self.devise))
    }

    /// Montant opposé (exact, sans dépassement possible).
    #[must_use]
    pub fn oppose(&self) -> Montant {
        Self::nouveau(-self.valeur, self.devise)
    }

    /// Arrondi d'un montant en devise de tenue à l'échelle de cette devise : au plus près, la
    /// moitié s'éloignant de zéro.
    ///
    /// # Errors
    /// [`ErreurMontant::PasEnDeviseTenue`] si le montant n'est pas dans la devise de tenue.
    pub fn arrondi_tenue(&self, tenue: &DeviseTenue) -> Result<Montant, ErreurMontant> {
        if self.devise != tenue.devise {
            return Err(ErreurMontant::PasEnDeviseTenue(self.devise));
        }
        Ok(Self::nouveau(tenue.arrondir(self.valeur), tenue.devise))
    }

    /// Contre-valeur en devise de tenue, arrondie à son échelle (spec §6.6) : `valeur × cours`.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn contre_valeur_tenue(
        &self,
        cours: Cours,
        tenue: &DeviseTenue,
    ) -> Result<Montant, ErreurMontant> {
        let valeur = self
            .valeur
            .checked_mul(cours.valeur())
            .ok_or(ErreurMontant::Depassement)?;
        Ok(Self::nouveau(tenue.arrondir(valeur), tenue.devise))
    }

    fn meme_devise(&self, autre: &Montant) -> Result<(), ErreurMontant> {
        if self.devise == autre.devise {
            Ok(())
        } else {
            Err(ErreurMontant::DevisesDifferentes(self.devise, autre.devise))
        }
    }
}

impl fmt::Display for Montant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.valeur, self.devise)
    }
}

/// Cours de change : nombre d'unités de la devise de tenue pour une unité de devise.
/// Strictement positif.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cours(Decimal);

impl Cours {
    /// Construit un cours de change.
    ///
    /// # Errors
    /// [`ErreurMontant::CoursInvalide`] si le cours n'est pas strictement positif.
    pub fn nouveau(valeur: Decimal) -> Result<Self, ErreurMontant> {
        if valeur > Decimal::ZERO {
            Ok(Self(valeur))
        } else {
            Err(ErreurMontant::CoursInvalide(valeur))
        }
    }

    /// Analyse un cours écrit comme un montant (`655.957`).
    ///
    /// # Errors
    /// Format invalide ou cours non strictement positif.
    pub fn parse(texte: &str) -> Result<Self, ErreurMontant> {
        Self::nouveau(analyser_decimal(texte)?)
    }

    /// Valeur exacte du cours.
    #[must_use]
    pub fn valeur(&self) -> Decimal {
        self.0
    }
}

impl FromStr for Devise {
    type Err = ErreurMontant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Analyse stricte d'un décimal `-?chiffres(.chiffres)?`, au plus [`CHIFFRES_MAX`] chiffres.
fn analyser_decimal(texte: &str) -> Result<Decimal, ErreurMontant> {
    let invalide = || ErreurMontant::FormatInvalide(texte.to_owned());
    let corps = texte.strip_prefix('-').unwrap_or(texte);
    let (entier, decimales) = match corps.split_once('.') {
        Some((e, d)) => (e, Some(d)),
        None => (corps, None),
    };
    let chiffres = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !chiffres(entier) || decimales.is_some_and(|d| !chiffres(d)) {
        return Err(invalide());
    }
    if entier.len() + decimales.map_or(0, str::len) > CHIFFRES_MAX {
        return Err(invalide());
    }
    Decimal::from_str_exact(texte).map_err(|_| invalide())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn devise(code: &str) -> Devise {
        Devise::parse(code).unwrap()
    }

    fn tenue(code: &str, decimales: u8) -> DeviseTenue {
        DeviseTenue::nouvelle(devise(code), decimales).unwrap()
    }

    fn xof() -> DeviseTenue {
        tenue("XOF", 0)
    }

    fn cdf() -> DeviseTenue {
        tenue("CDF", 2)
    }

    fn m(texte: &str) -> Montant {
        Montant::parse(texte, devise("XOF")).unwrap()
    }

    #[test]
    fn devise_valide_et_invalide() {
        assert_eq!(Devise::parse("EUR").unwrap().code(), "EUR");
        assert_eq!(devise("XAF").to_string(), "XAF");
        for code in ["eur", "EU", "EURO", "E1R", "", "ÉUR"] {
            assert!(Devise::parse(code).is_err(), "{code}");
        }
    }

    #[test]
    fn echelle_de_la_devise_de_tenue_bornee() {
        assert!(DeviseTenue::nouvelle(devise("XOF"), 0).is_ok());
        assert!(DeviseTenue::nouvelle(devise("CLF"), 4).is_ok());
        assert_eq!(
            DeviseTenue::nouvelle(devise("XOF"), 5),
            Err(ErreurMontant::EchelleInvalide(5))
        );
    }

    #[test]
    fn analyse_stricte() {
        assert_eq!(m("1234.50").valeur().to_string(), "1234.50");
        assert_eq!(m("-7").valeur().to_string(), "-7");
        for texte in [
            "", "-", "+1", "1e5", "1.", ".5", "1 000", "1,5", "0x10", "1_000", " 1", "--1",
        ] {
            assert!(Montant::parse(texte, devise("XOF")).is_err(), "{texte:?}");
            assert!(Montant::parse_tenue(texte, &cdf()).is_err(), "{texte:?}");
        }
        assert!(Montant::parse(&"9".repeat(29), devise("XOF")).is_err());
    }

    #[test]
    fn analyse_selon_l_echelle_de_la_devise_de_tenue() {
        // XOF : aucune décimale significative.
        assert_eq!(Montant::parse_tenue("1500", &xof()).unwrap(), m("1500"));
        assert_eq!(
            Montant::parse_tenue("1500.00", &xof()).unwrap().valeur(),
            Decimal::from(1500)
        );
        assert_eq!(
            Montant::parse_tenue("1500.5", &xof()),
            Err(ErreurMontant::EchelleExcessive { decimales: 0 })
        );
        // CDF : deux décimales.
        let attendu = Montant::parse("1500.25", devise("CDF")).unwrap();
        assert_eq!(Montant::parse_tenue("1500.25", &cdf()).unwrap(), attendu);
        assert_eq!(
            Montant::parse_tenue("1500.255", &cdf()),
            Err(ErreurMontant::EchelleExcessive { decimales: 2 })
        );
        // Zéros finaux écrits : acceptés (la forme canonique les retire).
        assert!(Montant::parse_tenue("1500.250", &cdf()).is_ok());
        // La devise du montant est celle de la tenue.
        assert_eq!(
            Montant::parse_tenue("1", &cdf()).unwrap().devise(),
            devise("CDF")
        );
    }

    #[test]
    fn arrondi_moitie_eloignee_de_zero_selon_l_echelle() {
        let cas = [
            ("0.5", "1"),
            ("1.5", "2"),
            ("2.5", "3"),
            ("2.4999", "2"),
            ("-0.5", "-1"),
            ("-2.5", "-3"),
            ("100", "100"),
        ];
        for (entree, attendu) in cas {
            assert_eq!(
                m(entree).arrondi_tenue(&xof()).unwrap(),
                m(attendu),
                "{entree}"
            );
        }
        let cas_cdf = [
            ("0.005", "0.01"),
            ("1.004", "1.00"),
            ("-0.005", "-0.01"),
            ("2.675", "2.68"),
        ];
        for (entree, attendu) in cas_cdf {
            let entree = Montant::parse(entree, devise("CDF")).unwrap();
            assert_eq!(
                entree.arrondi_tenue(&cdf()).unwrap().valeur().to_string(),
                attendu
            );
        }
        let eur = Montant::parse("1.5", devise("EUR")).unwrap();
        assert!(matches!(
            eur.arrondi_tenue(&xof()),
            Err(ErreurMontant::PasEnDeviseTenue(_))
        ));
    }

    #[test]
    fn contre_valeur_selon_l_echelle() {
        let eur = devise("EUR");
        let cours = Cours::parse("655.957").unwrap();
        let montant = Montant::parse("100.00", eur).unwrap();
        assert_eq!(
            montant.contre_valeur_tenue(cours, &xof()).unwrap(),
            m("65596")
        );
        // En CDF : deux décimales conservées (100 × 2,5551 = 255,51).
        let cours = Cours::parse("2.5551").unwrap();
        let en_cdf = montant.contre_valeur_tenue(cours, &cdf()).unwrap();
        assert_eq!(en_cdf.valeur().to_string(), "255.51");
        assert_eq!(en_cdf.devise(), devise("CDF"));
        assert!(Cours::parse("0").is_err());
        assert!(Cours::parse("-1").is_err());
    }

    #[test]
    fn operations_entre_devises_refusees() {
        let eur = Montant::parse("1", devise("EUR")).unwrap();
        assert!(matches!(
            m("1").ajouter(&eur),
            Err(ErreurMontant::DevisesDifferentes(_, _))
        ));
        assert!(m("1").soustraire(&eur).is_err());
    }

    #[test]
    fn depassement_signale_sans_panique() {
        let max = Montant::nouveau(Decimal::MAX, devise("XOF"));
        assert_eq!(max.ajouter(&max), Err(ErreurMontant::Depassement));
        assert_eq!(
            max.oppose().soustraire(&max),
            Err(ErreurMontant::Depassement)
        );
        let cours = Cours::nouveau(Decimal::MAX).unwrap();
        assert_eq!(
            max.contre_valeur_tenue(cours, &xof()),
            Err(ErreurMontant::Depassement)
        );
    }

    fn decimal_quelconque() -> impl Strategy<Value = Decimal> {
        (any::<i64>(), 0u32..=6).prop_map(|(m, e)| Decimal::new(m, e))
    }

    proptest! {
        #[test]
        fn aller_retour_d_analyse(d in decimal_quelconque()) {
            let attendu = Montant::nouveau(d, devise("XOF"));
            let relu = Montant::parse(&d.to_string(), devise("XOF")).unwrap();
            prop_assert_eq!(relu, attendu);
            prop_assert_eq!(relu.valeur().scale(), d.scale());
        }

        #[test]
        fn addition_puis_soustraction_sans_perte(a in decimal_quelconque(), b in decimal_quelconque()) {
            let (a, b) = (Montant::nouveau(a, devise("XOF")), Montant::nouveau(b, devise("XOF")));
            let somme = a.ajouter(&b).unwrap();
            prop_assert_eq!(somme.soustraire(&b).unwrap(), a);
            prop_assert_eq!(a.ajouter(&a.oppose()).unwrap().est_nul(), true);
        }

        /// Arrondi par devise : à l'échelle de la tenue, au plus près, idempotent, et l'écart
        /// ne dépasse jamais une demi-unité de la plus petite unité de la devise.
        #[test]
        fn arrondi_au_plus_pres_par_devise(d in decimal_quelconque(), decimales in 0u8..=4) {
            let t = DeviseTenue::nouvelle(devise("ZZZ"), decimales).unwrap();
            let arrondi = Montant::nouveau(d, devise("ZZZ")).arrondi_tenue(&t).unwrap().valeur();
            prop_assert!(arrondi.scale() <= u32::from(decimales));
            let demi = Decimal::new(5, u32::from(decimales) + 1);
            prop_assert!((arrondi - d).abs() <= demi);
            prop_assert_eq!(t.arrondir(arrondi), arrondi);
        }

        /// Un montant arrondi à l'échelle de la tenue est toujours accepté par l'analyse de saisie
        /// de cette échelle ; un montant qui a une décimale de plus ne l'est jamais.
        #[test]
        fn analyse_de_saisie_et_echelle(unites in 0i64..1_000_000_000, decimales in 0u8..=4) {
            let t = DeviseTenue::nouvelle(devise("ZZZ"), decimales).unwrap();
            let valeur = Decimal::new(unites, u32::from(decimales));
            prop_assert!(Montant::parse_tenue(&valeur.to_string(), &t).is_ok());
            if decimales < 4 {
                // Dernier chiffre non nul à l'échelle suivante.
                let trop = Decimal::new(unites * 10 + 1, u32::from(decimales) + 1);
                prop_assert_eq!(
                    Montant::parse_tenue(&trop.to_string(), &t),
                    Err(ErreurMontant::EchelleExcessive { decimales })
                );
            }
        }
    }
}
