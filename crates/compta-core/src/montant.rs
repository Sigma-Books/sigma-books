//! Montants en décimal exact, devise attachée (Fondations §2.4, spec §5.5 et §6.6).
//!
//! Aucun flottant : la valeur est un [`Decimal`]. Les calculs internes sont exacts ; seule
//! la contre-valeur en XOF est arrondie au franc, au plus près, la moitié s'éloignant de zéro
//! (`0,5` → `1`, `-0,5` → `-1` : pas d'arrondi bancaire). Toute opération qui pourrait dépasser
//! la capacité du décimal renvoie une erreur au lieu de paniquer.

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
    /// Arrondi au franc demandé sur un montant qui n'est pas en XOF.
    #[error("arrondi au franc impossible : le montant est en {0}, pas en XOF")]
    PasEnXof(Devise),
    /// Un cours de change doit être strictement positif.
    #[error("cours de change invalide : {0} (strictement positif attendu)")]
    CoursInvalide(Decimal),
}

/// Code de devise ISO 4217 (trois lettres majuscules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Devise([u8; 3]);

impl Devise {
    /// Franc CFA (UEMOA), devise de tenue (spec §5.5).
    pub const XOF: Devise = Devise(*b"XOF");

    /// Analyse un code de devise. Seul constructeur pour une devise autre que XOF.
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
        let valeur = Decimal::from_str_exact(texte).map_err(|_| invalide())?;
        Ok(Self::nouveau(valeur, devise))
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

    /// Arrondi au franc d'un montant en XOF : au plus près, la moitié s'éloignant de zéro.
    ///
    /// # Errors
    /// [`ErreurMontant::PasEnXof`] si le montant n'est pas en XOF.
    pub fn arrondi_xof(&self) -> Result<Montant, ErreurMontant> {
        if self.devise != Devise::XOF {
            return Err(ErreurMontant::PasEnXof(self.devise));
        }
        Ok(Self::nouveau(arrondi_franc(self.valeur), Devise::XOF))
    }

    /// Contre-valeur en XOF, arrondie au franc (spec §6.6) : `valeur × cours`.
    ///
    /// # Errors
    /// Dépassement de capacité.
    pub fn contre_valeur_xof(&self, cours: Cours) -> Result<Montant, ErreurMontant> {
        let valeur = self
            .valeur
            .checked_mul(cours.valeur())
            .ok_or(ErreurMontant::Depassement)?;
        Ok(Self::nouveau(arrondi_franc(valeur), Devise::XOF))
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

/// Cours de change : nombre de francs CFA pour une unité de devise. Strictement positif.
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
        Self::nouveau(Montant::parse(texte, Devise::XOF)?.valeur())
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

fn arrondi_franc(valeur: Decimal) -> Decimal {
    valeur.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn xof(texte: &str) -> Montant {
        Montant::parse(texte, Devise::XOF).unwrap()
    }

    #[test]
    fn devise_valide_et_invalide() {
        assert_eq!(Devise::parse("EUR").unwrap().code(), "EUR");
        assert_eq!(Devise::XOF.to_string(), "XOF");
        for code in ["eur", "EU", "EURO", "E1R", "", "ÉUR"] {
            assert!(Devise::parse(code).is_err(), "{code}");
        }
    }

    #[test]
    fn analyse_stricte() {
        assert_eq!(xof("1234.50").valeur().to_string(), "1234.50");
        assert_eq!(xof("-7").valeur().to_string(), "-7");
        for texte in [
            "", "-", "+1", "1e5", "1.", ".5", "1 000", "1,5", "0x10", "1_000", " 1", "--1",
        ] {
            assert!(Montant::parse(texte, Devise::XOF).is_err(), "{texte:?}");
        }
        assert!(Montant::parse(&"9".repeat(29), Devise::XOF).is_err());
    }

    #[test]
    fn arrondi_au_franc_moitie_eloignee_de_zero() {
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
            assert_eq!(xof(entree).arrondi_xof().unwrap(), xof(attendu), "{entree}");
        }
        let eur = Montant::parse("1.5", Devise::parse("EUR").unwrap()).unwrap();
        assert!(matches!(eur.arrondi_xof(), Err(ErreurMontant::PasEnXof(_))));
    }

    #[test]
    fn contre_valeur_en_xof() {
        let eur = Devise::parse("EUR").unwrap();
        let cours = Cours::parse("655.957").unwrap();
        let m = Montant::parse("100.00", eur).unwrap();
        assert_eq!(m.contre_valeur_xof(cours).unwrap(), xof("65596"));
        assert!(Cours::parse("0").is_err());
        assert!(Cours::parse("-1").is_err());
    }

    #[test]
    fn operations_entre_devises_refusees() {
        let eur = Montant::parse("1", Devise::parse("EUR").unwrap()).unwrap();
        assert!(matches!(
            xof("1").ajouter(&eur),
            Err(ErreurMontant::DevisesDifferentes(_, _))
        ));
        assert!(xof("1").soustraire(&eur).is_err());
    }

    #[test]
    fn depassement_signale_sans_panique() {
        let max = Montant::nouveau(Decimal::MAX, Devise::XOF);
        assert_eq!(max.ajouter(&max), Err(ErreurMontant::Depassement));
        assert_eq!(
            max.oppose().soustraire(&max),
            Err(ErreurMontant::Depassement)
        );
        let cours = Cours::nouveau(Decimal::MAX).unwrap();
        assert_eq!(
            max.contre_valeur_xof(cours),
            Err(ErreurMontant::Depassement)
        );
    }

    fn decimal_quelconque() -> impl Strategy<Value = Decimal> {
        (any::<i64>(), 0u32..=6).prop_map(|(m, e)| Decimal::new(m, e))
    }

    proptest! {
        #[test]
        fn aller_retour_d_analyse(d in decimal_quelconque()) {
            let m = Montant::nouveau(d, Devise::XOF);
            let relu = Montant::parse(&d.to_string(), Devise::XOF).unwrap();
            prop_assert_eq!(relu, m);
            prop_assert_eq!(relu.valeur().scale(), d.scale());
        }

        #[test]
        fn addition_puis_soustraction_sans_perte(a in decimal_quelconque(), b in decimal_quelconque()) {
            let (a, b) = (Montant::nouveau(a, Devise::XOF), Montant::nouveau(b, Devise::XOF));
            let somme = a.ajouter(&b).unwrap();
            prop_assert_eq!(somme.soustraire(&b).unwrap(), a);
            prop_assert_eq!(a.ajouter(&a.oppose()).unwrap().est_nul(), true);
        }

        #[test]
        fn arrondi_au_plus_pres(d in decimal_quelconque()) {
            let arrondi = Montant::nouveau(d, Devise::XOF).arrondi_xof().unwrap().valeur();
            prop_assert_eq!(arrondi.scale(), 0);
            prop_assert!((arrondi - d).abs() <= Decimal::new(5, 1));
        }
    }
}
