//! Comptes de trésorerie des journaux par défaut d'un dossier (spec §5.2).
//!
//! `compta-core` décrit les journaux par défaut sans aucun numéro de compte
//! ([`compta_core::journaux_par_defaut`]). Le compte de chaque journal de trésorerie par défaut
//! (CA : caisse) est une **donnée** de ce crate, `data/journaux.toml`, qui cite sa source.
//! [`Referentiel::journaux_par_defaut`] instancie les journaux d'un nouveau dossier.

use compta_core::{
    CodeJournal, ErreurJournal, Journaux, NumeroCompte, PlanComptes, journaux_par_defaut,
};
use serde::Deserialize;

use crate::{ErreurReferentiel, Referentiel};

/// Données embarquées : compte des journaux de trésorerie par défaut.
const JOURNAUX_TOML: &str = include_str!("../data/journaux.toml");

/// Compte de trésorerie d'un journal par défaut, avec sa source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompteDeJournal {
    code: CodeJournal,
    compte: NumeroCompte,
    source: String,
}

impl CompteDeJournal {
    /// Code du journal par défaut.
    #[must_use]
    pub fn code(&self) -> &CodeJournal {
        &self.code
    }

    /// Compte de trésorerie (compte officiel).
    #[must_use]
    pub fn compte(&self) -> &NumeroCompte {
        &self.compte
    }

    /// Source (document et page).
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Brut {
    journal: Vec<JournalBrut>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalBrut {
    code: String,
    compte: String,
    source: String,
    motif: String,
}

impl Referentiel {
    /// Comptes des journaux de trésorerie par défaut, lus de `data/journaux.toml` et validés
    /// contre ce référentiel.
    ///
    /// # Errors
    /// [`ErreurReferentiel`] si les données embarquées sont invalides (garanti par les tests).
    pub fn comptes_des_journaux(&self) -> Result<Vec<CompteDeJournal>, ErreurReferentiel> {
        self.comptes_des_journaux_depuis_toml(JOURNAUX_TOML)
    }

    /// Comptes des journaux au format de `data/journaux.toml` : codes valides et uniques,
    /// comptes de trésorerie de la liste officielle sans subdivision, source et motif
    /// renseignés.
    ///
    /// # Errors
    /// [`ErreurReferentiel`] décrivant la première anomalie rencontrée.
    pub fn comptes_des_journaux_depuis_toml(
        &self,
        texte_toml: &str,
    ) -> Result<Vec<CompteDeJournal>, ErreurReferentiel> {
        let brut: Brut =
            toml::from_str(texte_toml).map_err(|e| ErreurReferentiel::Syntaxe(e.to_string()))?;
        let mut comptes: Vec<CompteDeJournal> = Vec::new();
        for j in brut.journal {
            let code = CodeJournal::parse(&j.code).map_err(ErreurReferentiel::Journal)?;
            if j.source.trim().is_empty() || j.motif.trim().is_empty() {
                return Err(ErreurReferentiel::SourceManquante(code));
            }
            let compte =
                NumeroCompte::parse(&j.compte).map_err(|erreur| ErreurReferentiel::Numero {
                    numero: j.compte.clone(),
                    erreur,
                })?;
            if self.compte(&compte).is_none() {
                return Err(ErreurReferentiel::CompteDeJournalInconnu { code, compte });
            }
            if !self.est_de_tresorerie(&compte) {
                return Err(ErreurReferentiel::CompteDeJournalHorsTresorerie { code, compte });
            }
            if self.a_des_subdivisions(&compte) {
                return Err(ErreurReferentiel::CompteDeJournalSubdivise { code, compte });
            }
            if comptes.iter().any(|c| c.code == code) {
                return Err(ErreurReferentiel::Journal(ErreurJournal::Doublon(code)));
            }
            comptes.push(CompteDeJournal {
                code,
                compte,
                source: j.source,
            });
        }
        Ok(comptes)
    }

    /// Journaux par défaut d'un nouveau dossier ([`compta_core::journaux_par_defaut`]), chaque
    /// journal de trésorerie recevant son compte de [`Referentiel::comptes_des_journaux`].
    ///
    /// # Errors
    /// Données invalides, journal de trésorerie par défaut sans compte, compte attribué à un
    /// journal qui n'est pas un journal de trésorerie par défaut, ou compte qui n'est pas un
    /// compte de mouvement actif du `plan`.
    pub fn journaux_par_defaut(&self, plan: &PlanComptes) -> Result<Journaux, ErreurReferentiel> {
        let comptes = self.comptes_des_journaux()?;
        instancier(&comptes, plan)
    }

    fn a_des_subdivisions(&self, numero: &NumeroCompte) -> bool {
        self.comptes
            .keys()
            .any(|n| n.parent().as_ref() == Some(numero))
    }
}

/// Instancie les journaux par défaut avec les comptes donnés.
fn instancier(
    comptes: &[CompteDeJournal],
    plan: &PlanComptes,
) -> Result<Journaux, ErreurReferentiel> {
    let modeles = journaux_par_defaut().map_err(ErreurReferentiel::Journal)?;
    if let Some(orphelin) = comptes
        .iter()
        .find(|c| !modeles.iter().any(|m| m.code() == &c.code))
    {
        return Err(ErreurReferentiel::Journal(ErreurJournal::Inconnu(
            orphelin.code.clone(),
        )));
    }
    let mut journaux = Journaux::new();
    for modele in modeles {
        let compte = comptes
            .iter()
            .find(|c| &c.code == modele.code())
            .map(|c| c.compte.clone());
        let journal = modele
            .instancier(compte, plan)
            .map_err(ErreurReferentiel::Journal)?;
        journaux
            .ajouter(journal)
            .map_err(ErreurReferentiel::Journal)?;
    }
    Ok(journaux)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compta_core::{ErreurPlan, TexteCourt, TypeJournal};

    fn n(numero: &str) -> NumeroCompte {
        NumeroCompte::parse(numero).unwrap()
    }

    fn c(code: &str) -> CodeJournal {
        CodeJournal::parse(code).unwrap()
    }

    fn officiel() -> Referentiel {
        Referentiel::officiel().unwrap()
    }

    #[test]
    fn caisse_sur_5711_en_monnaie_nationale() {
        // AUDCIF, liste des comptes, p. PDF 252 : 571 se subdivise en 5711 et 5712 ; on ne
        // saisit que sur un compte sans subdivision.
        let r = officiel();
        let comptes = r.comptes_des_journaux().unwrap();
        assert_eq!(comptes.len(), 1);
        assert_eq!(comptes[0].code(), &c("CA"));
        assert_eq!(comptes[0].compte(), &n("5711"));
        assert!(comptes[0].source().contains("p. PDF 252"));
        assert_eq!(r.compte(&n("5711")).unwrap().page(), 252);

        // Le plan officiel instancié rend 5711 de mouvement et 571 de regroupement.
        let plan = r.plan_comptes().unwrap();
        assert!(plan.compte_de_mouvement_actif(&n("5711")).is_ok());
        assert_eq!(
            plan.compte_de_mouvement_actif(&n("571")).unwrap_err(),
            ErreurPlan::Regroupement(n("571"))
        );
    }

    #[test]
    fn journaux_par_defaut_d_un_nouveau_dossier() {
        let r = officiel();
        let plan = r.plan_comptes().unwrap();
        let journaux = r.journaux_par_defaut(&plan).unwrap();
        let codes: Vec<&str> = journaux.journaux().map(|j| j.code().as_str()).collect();
        assert_eq!(codes, ["AC", "AN", "CA", "IN", "OD", "PA", "VT"]);
        // Chaque journal de trésorerie par défaut a reçu son compte : CA seul.
        for journal in journaux.journaux() {
            let attendu = (journal.genre() == TypeJournal::Tresorerie).then(|| n("5711"));
            assert_eq!(
                journal.compte_tresorerie(),
                attendu.as_ref(),
                "{}",
                journal.code()
            );
        }
        assert_eq!(journaux.verifier_plan(&plan), Ok(()));

        // Un sous-compte sous 5711 rendrait le journal CA inutilisable : refusé.
        let mut candidat = plan.clone();
        candidat
            .ajouter_sous_compte(n("5711-A"), TexteCourt::parse("Caisse A").unwrap())
            .unwrap();
        assert_eq!(
            journaux.verifier_plan(&candidat),
            Err(ErreurJournal::CompteTresorerieRetire(
                c("CA"),
                ErreurPlan::Regroupement(n("5711"))
            ))
        );
    }

    #[test]
    fn donnees_invalides() {
        let r = officiel();
        let ligne = |code: &str, compte: &str, source: &str| {
            format!(
                "[[journal]]\ncode = \"{code}\"\ncompte = \"{compte}\"\nsource = \"{source}\"\nmotif = \"m\"\n"
            )
        };
        let lire = |texte: &str| r.comptes_des_journaux_depuis_toml(texte);
        assert!(matches!(
            lire("[[journal]]\ncode = \"CA\""),
            Err(ErreurReferentiel::Syntaxe(_))
        ));
        assert!(matches!(
            lire(&ligne("ca", "5711", "p. 252")),
            Err(ErreurReferentiel::Journal(ErreurJournal::CodeInvalide(_)))
        ));
        assert_eq!(
            lire(&ligne("CA", "5711", " ")),
            Err(ErreurReferentiel::SourceManquante(c("CA")))
        );
        assert!(matches!(
            lire(&ligne("CA", "57x", "p. 252")),
            Err(ErreurReferentiel::Numero { .. })
        ));
        assert_eq!(
            lire(&ligne("CA", "5719", "p. 252")),
            Err(ErreurReferentiel::CompteDeJournalInconnu {
                code: c("CA"),
                compte: n("5719")
            })
        );
        assert_eq!(
            lire(&ligne("CA", "5711-A", "p. 252")),
            Err(ErreurReferentiel::CompteDeJournalInconnu {
                code: c("CA"),
                compte: n("5711-A")
            })
        );
        assert_eq!(
            lire(&ligne("CA", "571", "p. 252")),
            Err(ErreurReferentiel::CompteDeJournalSubdivise {
                code: c("CA"),
                compte: n("571")
            })
        );
        // Compte de la liste officielle, sans subdivision, hors de la classe de trésorerie.
        let hors_tresorerie = r
            .comptes()
            .map(|compte| compte.numero().clone())
            .find(|numero| !r.est_de_tresorerie(numero) && !r.a_des_subdivisions(numero))
            .unwrap();
        assert_eq!(
            lire(&ligne("CA", hors_tresorerie.as_str(), "p. 252")),
            Err(ErreurReferentiel::CompteDeJournalHorsTresorerie {
                code: c("CA"),
                compte: hors_tresorerie
            })
        );
        let double = format!(
            "{}{}",
            ligne("CA", "5711", "p. 252"),
            ligne("CA", "5712", "p. 252")
        );
        assert_eq!(
            lire(&double),
            Err(ErreurReferentiel::Journal(ErreurJournal::Doublon(c("CA"))))
        );
    }

    #[test]
    fn instanciation_refusee() {
        let r = officiel();
        let plan = r.plan_comptes().unwrap();
        let compte = |code: &str, numero: &str| CompteDeJournal {
            code: c(code),
            compte: n(numero),
            source: "p. 252".to_owned(),
        };
        // Journal de trésorerie par défaut sans compte.
        assert_eq!(
            instancier(&[], &plan),
            Err(ErreurReferentiel::Journal(
                ErreurJournal::CompteTresorerieManquant(c("CA"))
            ))
        );
        // Compte attribué à un journal qui n'est pas de trésorerie, ou inconnu.
        assert_eq!(
            instancier(&[compte("CA", "5711"), compte("AC", "5712")], &plan),
            Err(ErreurReferentiel::Journal(ErreurJournal::CompteNonAttendu(
                c("AC")
            )))
        );
        assert_eq!(
            instancier(&[compte("CA", "5711"), compte("BQ1", "5211")], &plan),
            Err(ErreurReferentiel::Journal(ErreurJournal::Inconnu(c("BQ1"))))
        );
        // Compte de mouvement du plan hors de la classe de trésorerie.
        let hors_tresorerie = plan
            .comptes()
            .map(|compte| compte.numero().clone())
            .find(|numero| {
                plan.compte_de_mouvement_actif(numero)
                    .is_ok_and(|compte| !compte.est_de_tresorerie())
            })
            .unwrap();
        assert_eq!(
            instancier(
                &[CompteDeJournal {
                    code: c("CA"),
                    compte: hors_tresorerie.clone(),
                    source: "p. 252".to_owned(),
                }],
                &plan
            ),
            Err(ErreurReferentiel::Journal(
                ErreurJournal::CompteHorsTresorerie(c("CA"), hors_tresorerie)
            ))
        );
        // Compte désactivé dans le plan du dossier.
        let mut inactif = plan.clone();
        inactif.desactiver(&n("5711")).unwrap();
        assert_eq!(
            instancier(&[compte("CA", "5711")], &inactif),
            Err(ErreurReferentiel::Journal(ErreurJournal::CompteTresorerie(
                c("CA"),
                ErreurPlan::Inactif(n("5711"))
            )))
        );
    }
}
