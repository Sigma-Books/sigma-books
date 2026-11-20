//! Plan de comptes d'un dossier (spec §5.1).
//!
//! Le plan est construit à partir du **référentiel officiel passé en paramètre**
//! ([`CompteReference`], fourni par le crate `syscohada`) : ce module ne contient aucun numéro
//! de compte. Le dossier y ajoute des sous-comptes libres et des comptes auxiliaires (tiers),
//! toujours sous un compte existant ; le tronc officiel est donc obligatoire et immuable.
//!
//! - Un compte **de regroupement** a au moins un compte enfant dans le plan ; un compte
//!   **de mouvement** n'en a aucun. Seuls les comptes de mouvement actifs reçoivent des
//!   écritures (spec §6.2, invariant 3). Créer un sous-compte sous un compte **déjà
//!   mouvementé** est autorisé (décision du mainteneur, 2026-11-17, `docs/lots/L1.md`) : le
//!   compte devient un compte de regroupement, ses écritures passées restent telles quelles et
//!   les nouvelles vont sur ses sous-comptes.
//! - Un compte est **collectif** si lui-même ou l'un de ses ascendants officiels est marqué
//!   collectif dans le référentiel (401, 411, 421… : la liste vient des données).
//!   Ce caractère n'est modifiable par aucune fonction : il ne peut donc pas changer une fois
//!   le compte mouvementé.
//! - Un compte officiel est **admis en journal de trésorerie** si le référentiel le marque
//!   comme tel, compte par compte (la liste vient des données, qui peuvent exclure un
//!   sous-compte officiel d'un compte admis) ; un compte créé par le dossier hérite le caractère
//!   de son parent. Seul un tel compte peut être le compte d'un journal de trésorerie ; ce
//!   caractère n'est, lui non plus, modifiable par aucune fonction.
//! - Un compte ayant des écritures ne peut être supprimé, seulement désactivé. Le plan ne
//!   connaît pas les écritures : l'appelant lui indique l'[`Usage`] du compte.

use std::collections::BTreeMap;
use std::ops::Bound;

use crate::compte::NumeroCompte;
use crate::texte::TexteCourt;

/// Sens normal du solde d'un compte, tel que donné par le référentiel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensNormal {
    /// Solde normalement débiteur.
    Debiteur,
    /// Solde normalement créditeur.
    Crediteur,
    /// Solde débiteur ou créditeur selon les cas.
    Variable,
}

/// Compte du référentiel officiel, donnée d'entrée du plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteReference {
    numero: NumeroCompte,
    intitule: TexteCourt,
    sens: SensNormal,
    collectif: bool,
    journal_tresorerie: bool,
}

impl CompteReference {
    /// Décrit un compte officiel.
    ///
    /// # Errors
    /// [`ErreurPlan::NonOfficiel`] si le numéro comporte un segment libre.
    pub fn nouveau(
        numero: NumeroCompte,
        intitule: TexteCourt,
        sens: SensNormal,
        collectif: bool,
    ) -> Result<Self, ErreurPlan> {
        if !numero.est_officiel() {
            return Err(ErreurPlan::NonOfficiel(numero));
        }
        Ok(Self {
            numero,
            intitule,
            sens,
            collectif,
            journal_tresorerie: false,
        })
    }

    /// Admet le compte comme compte d'un journal de trésorerie, d'après le référentiel. Seul
    /// ce compte est marqué : ses sous-comptes officiels portent leur propre marque, ses
    /// sous-comptes créés par le dossier en héritent.
    #[must_use]
    pub fn admettre_en_journal_de_tresorerie(mut self) -> Self {
        self.journal_tresorerie = true;
        self
    }

    /// Numéro officiel.
    #[must_use]
    pub fn numero(&self) -> &NumeroCompte {
        &self.numero
    }
}

/// Origine d'un compte du plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenreCompte {
    /// Compte du référentiel officiel.
    Officiel,
    /// Sous-compte libre créé par le dossier.
    SousCompte,
    /// Compte auxiliaire de tiers, sous un compte collectif.
    Auxiliaire,
}

/// Usage d'un compte dans les écritures, fourni par l'appelant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Usage {
    /// Aucune écriture (brouillard ou validée) ne mouvemente le compte.
    SansEcriture,
    /// Au moins une écriture mouvemente le compte.
    Mouvemente,
}

/// Compte du plan d'un dossier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compte {
    numero: NumeroCompte,
    intitule: TexteCourt,
    genre: GenreCompte,
    sens: SensNormal,
    collectif: bool,
    journal_tresorerie: bool,
    actif: bool,
}

impl Compte {
    /// Numéro du compte (immuable).
    #[must_use]
    pub fn numero(&self) -> &NumeroCompte {
        &self.numero
    }

    /// Intitulé.
    #[must_use]
    pub fn intitule(&self) -> &TexteCourt {
        &self.intitule
    }

    /// Origine du compte.
    #[must_use]
    pub fn genre(&self) -> GenreCompte {
        self.genre
    }

    /// Sens normal du solde (hérité du compte parent pour un compte créé par le dossier).
    #[must_use]
    pub fn sens(&self) -> SensNormal {
        self.sens
    }

    /// Vrai si le compte est collectif (lui-même ou par un ascendant).
    #[must_use]
    pub fn est_collectif(&self) -> bool {
        self.collectif
    }

    /// Vrai si le compte peut être le compte d'un journal de trésorerie (lui-même ou par un
    /// ascendant).
    #[must_use]
    pub fn est_admis_en_journal_de_tresorerie(&self) -> bool {
        self.journal_tresorerie
    }

    /// Vrai si le compte est actif.
    #[must_use]
    pub fn est_actif(&self) -> bool {
        self.actif
    }
}

/// Erreurs sur le plan de comptes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurPlan {
    /// Un compte du référentiel doit être officiel (sans segment libre).
    #[error("le compte {0} n'est pas un numéro officiel")]
    NonOfficiel(NumeroCompte),
    /// Un compte créé par le dossier doit porter au moins un segment libre.
    #[error("le compte {0} est un numéro officiel : il ne peut pas être créé par le dossier")]
    Officiel(NumeroCompte),
    /// Le numéro existe déjà.
    #[error("le compte {0} existe déjà")]
    Doublon(NumeroCompte),
    /// Le compte parent n'existe pas dans le plan.
    #[error("le compte parent de {0} n'existe pas")]
    ParentAbsent(NumeroCompte),
    /// Compte inconnu du plan.
    #[error("le compte {0} n'existe pas dans le plan")]
    Inconnu(NumeroCompte),
    /// Compte désactivé.
    #[error("le compte {0} est désactivé")]
    Inactif(NumeroCompte),
    /// Compte de regroupement : il a des comptes enfants.
    #[error("le compte {0} est un compte de regroupement")]
    Regroupement(NumeroCompte),
    /// Un compte auxiliaire doit être rattaché à un compte collectif.
    #[error("le compte {0} n'est pas rattaché à un compte collectif")]
    NonCollectif(NumeroCompte),
    /// Les comptes officiels ne sont ni renommés ni supprimés.
    #[error("le compte officiel {0} ne peut être ni renommé ni supprimé")]
    CompteOfficielImmuable(NumeroCompte),
    /// Un compte mouvementé ne peut être que désactivé.
    #[error("le compte {0} a des écritures : il peut être désactivé, pas supprimé")]
    CompteMouvemente(NumeroCompte),
}

/// Plan de comptes d'un dossier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanComptes {
    comptes: BTreeMap<NumeroCompte, Compte>,
}

impl PlanComptes {
    /// Construit le plan à partir du référentiel officiel.
    ///
    /// # Errors
    /// Doublon, ou compte dont le parent (numéro sans son dernier chiffre) est absent.
    pub fn depuis_referentiel(
        referentiel: impl IntoIterator<Item = CompteReference>,
    ) -> Result<Self, ErreurPlan> {
        let mut comptes = BTreeMap::new();
        for reference in referentiel {
            let compte = Compte {
                numero: reference.numero.clone(),
                intitule: reference.intitule,
                genre: GenreCompte::Officiel,
                sens: reference.sens,
                collectif: reference.collectif,
                journal_tresorerie: reference.journal_tresorerie,
                actif: true,
            };
            if comptes.insert(reference.numero.clone(), compte).is_some() {
                return Err(ErreurPlan::Doublon(reference.numero));
            }
        }
        // Héritage du caractère collectif et contrôle des parents, du plus court au plus long.
        // Le caractère « admis en journal de trésorerie » d'un compte officiel vient tel quel
        // du référentiel (une exclusion explicite l'y emporte sur l'héritage).
        let mut numeros: Vec<NumeroCompte> = comptes.keys().cloned().collect();
        numeros.sort_by_key(|n| n.as_str().len());
        for numero in numeros {
            if let Some(parent) = numero.parent() {
                let collectif_parent = comptes
                    .get(&parent)
                    .ok_or_else(|| ErreurPlan::ParentAbsent(numero.clone()))?
                    .collectif;
                if let Some(compte) = comptes.get_mut(&numero) {
                    compte.collectif |= collectif_parent;
                }
            }
        }
        Ok(Self { comptes })
    }

    /// Compte du plan, s'il existe.
    #[must_use]
    pub fn compte(&self, numero: &NumeroCompte) -> Option<&Compte> {
        self.comptes.get(numero)
    }

    /// Tous les comptes, par ordre de numéro.
    pub fn comptes(&self) -> impl Iterator<Item = &Compte> {
        self.comptes.values()
    }

    /// Vrai si le compte a au moins un enfant dans le plan.
    #[must_use]
    pub fn a_des_enfants(&self, numero: &NumeroCompte) -> bool {
        // Les descendants partagent le préfixe du numéro et se suivent dans l'ordre du plan.
        self.comptes
            .range::<NumeroCompte, _>((Bound::Excluded(numero), Bound::Unbounded))
            .take_while(|(n, _)| n.as_str().starts_with(numero.as_str()))
            .any(|(n, _)| n.parent().as_ref() == Some(numero))
    }

    /// Le compte peut-il recevoir une écriture ? Il doit exister, être actif et être un compte
    /// de mouvement (spec §6.2, invariant 3).
    ///
    /// # Errors
    /// [`ErreurPlan::Inconnu`], [`ErreurPlan::Inactif`] ou [`ErreurPlan::Regroupement`].
    pub fn compte_de_mouvement_actif(&self, numero: &NumeroCompte) -> Result<&Compte, ErreurPlan> {
        let compte = self
            .comptes
            .get(numero)
            .ok_or_else(|| ErreurPlan::Inconnu(numero.clone()))?;
        if !compte.actif {
            return Err(ErreurPlan::Inactif(numero.clone()));
        }
        if self.a_des_enfants(numero) {
            return Err(ErreurPlan::Regroupement(numero.clone()));
        }
        Ok(compte)
    }

    /// Ajoute un sous-compte libre (`6052-01`, `521-BICIS`) sous un compte existant.
    ///
    /// # Errors
    /// Numéro officiel, doublon ou parent absent.
    pub fn ajouter_sous_compte(
        &mut self,
        numero: NumeroCompte,
        intitule: TexteCourt,
    ) -> Result<&Compte, ErreurPlan> {
        self.ajouter(numero, intitule, GenreCompte::SousCompte)
    }

    /// Ajoute un compte auxiliaire de tiers (`401-SENELEC`) sous un compte collectif.
    ///
    /// # Errors
    /// Numéro officiel, doublon, parent absent ou non collectif.
    pub fn ajouter_auxiliaire(
        &mut self,
        numero: NumeroCompte,
        intitule: TexteCourt,
    ) -> Result<&Compte, ErreurPlan> {
        self.ajouter(numero, intitule, GenreCompte::Auxiliaire)
    }

    fn ajouter(
        &mut self,
        numero: NumeroCompte,
        intitule: TexteCourt,
        genre: GenreCompte,
    ) -> Result<&Compte, ErreurPlan> {
        if numero.est_officiel() {
            return Err(ErreurPlan::Officiel(numero));
        }
        if self.comptes.contains_key(&numero) {
            return Err(ErreurPlan::Doublon(numero));
        }
        let parent = numero
            .parent()
            .and_then(|p| self.comptes.get(&p))
            .ok_or_else(|| ErreurPlan::ParentAbsent(numero.clone()))?;
        if genre == GenreCompte::Auxiliaire && !parent.collectif {
            return Err(ErreurPlan::NonCollectif(numero));
        }
        let compte = Compte {
            numero: numero.clone(),
            intitule,
            genre,
            sens: parent.sens,
            collectif: parent.collectif,
            journal_tresorerie: parent.journal_tresorerie,
            actif: true,
        };
        Ok(self.comptes.entry(numero).or_insert(compte))
    }

    /// Renomme un compte créé par le dossier. Les intitulés officiels ne changent pas.
    ///
    /// # Errors
    /// Compte inconnu ou officiel.
    pub fn renommer(
        &mut self,
        numero: &NumeroCompte,
        intitule: TexteCourt,
    ) -> Result<(), ErreurPlan> {
        let compte = self.compte_modifiable(numero)?;
        compte.intitule = intitule;
        Ok(())
    }

    /// Désactive un compte : il ne reçoit plus d'écriture mais reste dans le plan et les états.
    ///
    /// # Errors
    /// Compte inconnu.
    pub fn desactiver(&mut self, numero: &NumeroCompte) -> Result<(), ErreurPlan> {
        self.definir_actif(numero, false)
    }

    /// Réactive un compte désactivé.
    ///
    /// # Errors
    /// Compte inconnu.
    pub fn reactiver(&mut self, numero: &NumeroCompte) -> Result<(), ErreurPlan> {
        self.definir_actif(numero, true)
    }

    /// Supprime un compte créé par le dossier, sans enfant et sans écriture.
    ///
    /// # Errors
    /// Compte inconnu, officiel, de regroupement ou mouvementé.
    pub fn supprimer(&mut self, numero: &NumeroCompte, usage: Usage) -> Result<(), ErreurPlan> {
        self.compte_modifiable(numero)?;
        if self.a_des_enfants(numero) {
            return Err(ErreurPlan::Regroupement(numero.clone()));
        }
        if usage == Usage::Mouvemente {
            return Err(ErreurPlan::CompteMouvemente(numero.clone()));
        }
        self.comptes.remove(numero);
        Ok(())
    }

    fn definir_actif(&mut self, numero: &NumeroCompte, actif: bool) -> Result<(), ErreurPlan> {
        let compte = self
            .comptes
            .get_mut(numero)
            .ok_or_else(|| ErreurPlan::Inconnu(numero.clone()))?;
        compte.actif = actif;
        Ok(())
    }

    fn compte_modifiable(&mut self, numero: &NumeroCompte) -> Result<&mut Compte, ErreurPlan> {
        let compte = self
            .comptes
            .get_mut(numero)
            .ok_or_else(|| ErreurPlan::Inconnu(numero.clone()))?;
        if compte.genre == GenreCompte::Officiel {
            return Err(ErreurPlan::CompteOfficielImmuable(numero.clone()));
        }
        Ok(compte)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;

    pub(crate) fn n(texte: &str) -> NumeroCompte {
        NumeroCompte::parse(texte).unwrap()
    }

    pub(crate) fn t(texte: &str) -> TexteCourt {
        TexteCourt::parse(texte).unwrap()
    }

    /// Référentiel fictif de test (numéros arbitraires, sans valeur normative). Le dernier
    /// champ admet le compte en journal de trésorerie (ses sous-comptes créés en héritent).
    pub(crate) fn plan_de_test() -> PlanComptes {
        let refs = [
            ("40", SensNormal::Crediteur, false, false),
            ("401", SensNormal::Crediteur, true, false),
            ("4011", SensNormal::Crediteur, false, false),
            ("52", SensNormal::Debiteur, false, true),
            ("521", SensNormal::Debiteur, false, true),
            ("60", SensNormal::Debiteur, false, false),
            ("605", SensNormal::Debiteur, false, false),
            ("6052", SensNormal::Debiteur, false, false),
        ];
        PlanComptes::depuis_referentiel(refs.into_iter().map(
            |(num, sens, collectif, tresorerie)| {
                let reference =
                    CompteReference::nouveau(n(num), t("Intitulé"), sens, collectif).unwrap();
                if tresorerie {
                    reference.admettre_en_journal_de_tresorerie()
                } else {
                    reference
                }
            },
        ))
        .unwrap()
    }

    /// Premier compte de mouvement actif du plan admis en journal de trésorerie (aucun numéro
    /// écrit en dur).
    pub(crate) fn compte_de_tresorerie(plan: &PlanComptes) -> NumeroCompte {
        plan.comptes()
            .map(Compte::numero)
            .find(|numero| {
                plan.compte(numero)
                    .is_some_and(Compte::est_admis_en_journal_de_tresorerie)
                    && plan.compte_de_mouvement_actif(numero).is_ok()
            })
            .unwrap()
            .clone()
    }

    #[test]
    fn construction_et_controles_du_referentiel() {
        let plan = plan_de_test();
        assert_eq!(plan.comptes().count(), 8);
        assert!(
            plan.compte(&n("4011")).unwrap().est_collectif(),
            "hérité de 401"
        );
        assert!(!plan.compte(&n("40")).unwrap().est_collectif());
        assert!(
            plan.compte(&n("521"))
                .unwrap()
                .est_admis_en_journal_de_tresorerie(),
            "marqué dans le référentiel"
        );
        assert!(
            !plan
                .compte(&n("6052"))
                .unwrap()
                .est_admis_en_journal_de_tresorerie()
        );

        let doublon = [
            CompteReference::nouveau(n("60"), t("A"), SensNormal::Debiteur, false).unwrap(),
            CompteReference::nouveau(n("60"), t("B"), SensNormal::Debiteur, false).unwrap(),
        ];
        assert_eq!(
            PlanComptes::depuis_referentiel(doublon),
            Err(ErreurPlan::Doublon(n("60")))
        );
        let orphelin =
            [CompteReference::nouveau(n("605"), t("A"), SensNormal::Debiteur, false).unwrap()];
        assert_eq!(
            PlanComptes::depuis_referentiel(orphelin),
            Err(ErreurPlan::ParentAbsent(n("605")))
        );
        assert_eq!(
            CompteReference::nouveau(n("60-A"), t("A"), SensNormal::Debiteur, false),
            Err(ErreurPlan::NonOfficiel(n("60-A")))
        );
    }

    #[test]
    fn marque_de_journal_de_tresorerie_par_compte_officiel() {
        // Le référentiel marque chaque compte officiel : un sous-compte officiel non marqué
        // d'un compte admis n'est pas admis ; un sous-compte créé par le dossier hérite.
        let mut plan = PlanComptes::depuis_referentiel([
            CompteReference::nouveau(n("52"), t("A"), SensNormal::Debiteur, false)
                .unwrap()
                .admettre_en_journal_de_tresorerie(),
            CompteReference::nouveau(n("521"), t("B"), SensNormal::Debiteur, false).unwrap(),
        ])
        .unwrap();
        assert!(
            plan.compte(&n("52"))
                .unwrap()
                .est_admis_en_journal_de_tresorerie()
        );
        assert!(
            !plan
                .compte(&n("521"))
                .unwrap()
                .est_admis_en_journal_de_tresorerie()
        );
        let cree = plan.ajouter_sous_compte(n("52-A"), t("C")).unwrap();
        assert!(cree.est_admis_en_journal_de_tresorerie());
        let cree = plan.ajouter_sous_compte(n("521-A"), t("D")).unwrap();
        assert!(!cree.est_admis_en_journal_de_tresorerie());
    }

    #[test]
    fn mouvement_et_regroupement() {
        let mut plan = plan_de_test();
        assert!(plan.compte_de_mouvement_actif(&n("6052")).is_ok());
        assert_eq!(
            plan.compte_de_mouvement_actif(&n("605")),
            Err(ErreurPlan::Regroupement(n("605")))
        );
        assert_eq!(
            plan.compte_de_mouvement_actif(&n("609")),
            Err(ErreurPlan::Inconnu(n("609")))
        );
        plan.ajouter_sous_compte(n("6052-01"), t("Eau")).unwrap();
        assert_eq!(
            plan.compte_de_mouvement_actif(&n("6052")),
            Err(ErreurPlan::Regroupement(n("6052")))
        );
        assert!(plan.compte_de_mouvement_actif(&n("6052-01")).is_ok());
        plan.desactiver(&n("6052-01")).unwrap();
        assert_eq!(
            plan.compte_de_mouvement_actif(&n("6052-01")),
            Err(ErreurPlan::Inactif(n("6052-01")))
        );
        plan.reactiver(&n("6052-01")).unwrap();
        assert!(plan.compte_de_mouvement_actif(&n("6052-01")).is_ok());
    }

    #[test]
    fn sous_comptes_et_auxiliaires() {
        let mut plan = plan_de_test();
        assert_eq!(
            plan.ajouter_sous_compte(n("6052"), t("X")).unwrap_err(),
            ErreurPlan::Officiel(n("6052"))
        );
        assert_eq!(
            plan.ajouter_sous_compte(n("609-01"), t("X")).unwrap_err(),
            ErreurPlan::ParentAbsent(n("609-01"))
        );
        assert_eq!(
            plan.ajouter_sous_compte(n("6052-01-A"), t("X"))
                .unwrap_err(),
            ErreurPlan::ParentAbsent(n("6052-01-A"))
        );
        let banque = plan
            .ajouter_sous_compte(n("521-BICIS"), t("Banque"))
            .unwrap();
        assert_eq!(banque.sens(), SensNormal::Debiteur);
        assert_eq!(banque.genre(), GenreCompte::SousCompte);
        assert!(banque.est_admis_en_journal_de_tresorerie(), "hérité de 521");
        assert_eq!(
            plan.ajouter_sous_compte(n("521-BICIS"), t("X"))
                .unwrap_err(),
            ErreurPlan::Doublon(n("521-BICIS"))
        );

        let fournisseur = plan
            .ajouter_auxiliaire(n("4011-SENELEC"), t("Fournisseur"))
            .unwrap();
        assert!(fournisseur.est_collectif());
        assert!(!fournisseur.est_admis_en_journal_de_tresorerie());
        assert_eq!(fournisseur.genre(), GenreCompte::Auxiliaire);
        assert_eq!(
            plan.ajouter_auxiliaire(n("6052-X"), t("X")).unwrap_err(),
            ErreurPlan::NonCollectif(n("6052-X"))
        );
    }

    #[test]
    fn suppression_et_renommage() {
        let mut plan = plan_de_test();
        plan.ajouter_sous_compte(n("6052-01"), t("Eau")).unwrap();
        plan.renommer(&n("6052-01"), t("Eau et assainissement"))
            .unwrap();
        assert_eq!(
            plan.compte(&n("6052-01")).unwrap().intitule().as_str(),
            "Eau et assainissement"
        );
        assert_eq!(
            plan.renommer(&n("6052"), t("X")),
            Err(ErreurPlan::CompteOfficielImmuable(n("6052")))
        );
        assert_eq!(
            plan.supprimer(&n("6052"), Usage::SansEcriture),
            Err(ErreurPlan::CompteOfficielImmuable(n("6052")))
        );
        assert_eq!(
            plan.supprimer(&n("6052-01"), Usage::Mouvemente),
            Err(ErreurPlan::CompteMouvemente(n("6052-01")))
        );
        plan.ajouter_sous_compte(n("6052-01-A"), t("Détail"))
            .unwrap();
        assert_eq!(
            plan.supprimer(&n("6052-01"), Usage::SansEcriture),
            Err(ErreurPlan::Regroupement(n("6052-01")))
        );
        plan.supprimer(&n("6052-01-A"), Usage::SansEcriture)
            .unwrap();
        plan.supprimer(&n("6052-01"), Usage::SansEcriture).unwrap();
        assert!(plan.compte(&n("6052-01")).is_none());
        assert!(plan.compte_de_mouvement_actif(&n("6052")).is_ok());
    }

    #[test]
    fn sous_compte_sous_un_compte_deja_mouvemente() {
        // Autorisé (décision du mainteneur, 2026-11-17) : le compte devient un compte de
        // regroupement ; il reste dans le plan avec ses écritures passées, ne peut plus en
        // recevoir et ne peut pas être supprimé.
        let mut plan = plan_de_test();
        plan.ajouter_sous_compte(n("521-BICIS"), t("Banque"))
            .unwrap();
        assert!(plan.compte_de_mouvement_actif(&n("521-BICIS")).is_ok());
        plan.ajouter_sous_compte(n("521-BICIS-EUR"), t("Banque, compte en euros"))
            .unwrap();
        assert_eq!(
            plan.compte_de_mouvement_actif(&n("521-BICIS")),
            Err(ErreurPlan::Regroupement(n("521-BICIS")))
        );
        assert!(plan.compte(&n("521-BICIS")).is_some());
        assert!(plan.compte_de_mouvement_actif(&n("521-BICIS-EUR")).is_ok());
        assert!(plan.supprimer(&n("521-BICIS"), Usage::Mouvemente).is_err());
        assert!(
            plan.supprimer(&n("521-BICIS"), Usage::SansEcriture)
                .is_err()
        );
    }

    proptest! {
        /// Quel que soit l'ordre des ajouts, le tronc de chaque compte du plan est un compte
        /// officiel, et la chaîne de ses ascendants est complète.
        #[test]
        fn tronc_toujours_officiel(segments in proptest::collection::vec("[A-Z0-9]{1,3}", 1..20)) {
            let mut plan = plan_de_test();
            for (i, segment) in segments.iter().enumerate() {
                let base = ["6052", "521", "4011"][i % 3];
                let numero = format!("{base}-{segment}");
                let _ = plan.ajouter_sous_compte(n(&numero), t("X"));
                let _ = plan.ajouter_sous_compte(n(&format!("{numero}-{i}")), t("Y"));
            }
            for compte in plan.comptes() {
                let tronc = n(compte.numero().tronc());
                prop_assert_eq!(plan.compte(&tronc).map(Compte::genre), Some(GenreCompte::Officiel));
                let mut courant = compte.numero().parent();
                while let Some(parent) = courant {
                    prop_assert!(plan.compte(&parent).is_some());
                    courant = parent.parent();
                }
            }
        }
    }
}
