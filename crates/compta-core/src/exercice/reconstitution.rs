//! Aplatissement et reconstitution des exercices relus de la persistance (lot L2).
//!
//! La base stocke une ligne par exercice et une ligne par période ([`LigneExercice`],
//! [`LignePeriode`]). [`Exercices::reconstituer`] reconstruit l'état à partir de ces lignes, sur
//! le modèle de [`crate::EcritureValidee::reconstituer`] : il ne **rejoue pas** les transitions
//! (qui revalideraient l'historique avec les règles du jour), il vérifie seulement que les
//! lignes décrivent un état que les transitions de [`Exercices`] peuvent atteindre.
//!
//! Contrôles structurels :
//! - bornes : code égal à l'année de clôture ; clôture au 31 décembre ; premier exercice
//!   conforme à l'AUDCIF art. 7 (même année, ou année suivante s'il débute au second
//!   semestre) ; exercices suivants contigus et calqués sur l'année civile ;
//! - périodes : exactement les mois civils de l'exercice, le premier éventuellement partiel ;
//! - statuts : au plus deux exercices non clôturés ; un exercice n'est jamais plus avancé
//!   qu'un exercice plus ancien ; un exercice clôturé a toutes ses périodes clôturées ; la
//!   dernière période d'un exercice non clôturé ne l'est pas ; les périodes clôturées forment
//!   un préfixe chronologique de toutes les périodes (AUDCIF art. 22 3°).
//!
//! L'ordre des lignes est indifférent. Aucune horloge : l'état relu ne dépend que des lignes.

use std::collections::BTreeMap;

use time::{Date, Month};

use super::{
    CodeExercice, EXERCICES_NON_CLOTURES_MAX, Exercice, Exercices, Periode, StatutExercice,
    StatutPeriode, decouper_en_mois,
};

/// Ligne plate d'un exercice, telle que stockée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneExercice {
    /// Code (année de clôture).
    pub code: CodeExercice,
    /// Premier jour.
    pub debut: Date,
    /// Dernier jour.
    pub fin: Date,
    /// Statut.
    pub statut: StatutExercice,
}

/// Ligne plate d'une période, telle que stockée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LignePeriode {
    /// Exercice auquel appartient la période.
    pub exercice: CodeExercice,
    /// Premier jour.
    pub debut: Date,
    /// Dernier jour.
    pub fin: Date,
    /// Statut.
    pub statut: StatutPeriode,
}

/// Motifs de refus d'une reconstitution ([`Exercices::reconstituer`]) : les lignes relues ne
/// décrivent pas un état atteignable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurReconstitutionExercices {
    /// Deux lignes d'exercice portent le même code.
    #[error("l'exercice {0} figure deux fois")]
    Doublon(CodeExercice),
    /// Le code n'est pas l'année de clôture.
    #[error("le code {code} n'est pas l'année de clôture de l'exercice terminé le {fin}")]
    CodeIncoherent {
        /// Code relu.
        code: CodeExercice,
        /// Dernier jour relu.
        fin: Date,
    },
    /// L'exercice ne se termine pas un 31 décembre (AUDCIF art. 7).
    #[error("l'exercice {code} se termine le {fin}, pas un 31 décembre (AUDCIF art. 7)")]
    FinNonCivile {
        /// Code de l'exercice.
        code: CodeExercice,
        /// Dernier jour relu.
        fin: Date,
    },
    /// Durée contraire à l'AUDCIF art. 7 : premier exercice de plus d'une année civile hors du
    /// cas du second semestre, ou exercice suivant qui n'est pas une année civile.
    #[error("l'exercice {code} (du {debut} au {fin}) ne respecte pas l'AUDCIF art. 7")]
    DureeNonConforme {
        /// Code de l'exercice.
        code: CodeExercice,
        /// Premier jour relu.
        debut: Date,
        /// Dernier jour relu.
        fin: Date,
    },
    /// Un exercice ne commence pas le lendemain de la clôture du précédent.
    #[error("l'exercice {suivant} ne commence pas le lendemain de la clôture de {precedent}")]
    Discontinuite {
        /// Exercice précédent.
        precedent: CodeExercice,
        /// Exercice suivant.
        suivant: CodeExercice,
    },
    /// Une période cite un exercice absent des lignes relues.
    #[error("la période du {debut} cite l'exercice {exercice}, absent")]
    PeriodeOrpheline {
        /// Exercice cité.
        exercice: CodeExercice,
        /// Premier jour de la période.
        debut: Date,
    },
    /// Les périodes d'un exercice ne sont pas ses mois civils.
    #[error("les périodes de l'exercice {0} ne sont pas ses mois civils")]
    DecoupageNonConforme(CodeExercice),
    /// Un exercice est plus avancé qu'un exercice plus ancien (clôture dans l'ordre).
    #[error("l'exercice {suivant} est plus avancé que l'exercice {anterieur}, plus ancien")]
    StatutsHorsOrdre {
        /// Exercice plus ancien.
        anterieur: CodeExercice,
        /// Exercice plus récent.
        suivant: CodeExercice,
    },
    /// Plus de deux exercices non clôturés.
    #[error("{0} exercices non clôturés : deux au plus")]
    TropDExercicesNonClotures(usize),
    /// Un exercice clôturé a une période non clôturée.
    #[error("l'exercice {exercice} est clôturé mais pas sa période du {debut}")]
    PeriodeNonClotureeDansExerciceCloture {
        /// Exercice clôturé.
        exercice: CodeExercice,
        /// Premier jour de la période.
        debut: Date,
    },
    /// La dernière période d'un exercice non clôturé est clôturée : elle se clôture avec lui.
    #[error("la dernière période de l'exercice {0}, non clôturé, est clôturée")]
    DernierePeriodeCloturee(CodeExercice),
    /// Une période est clôturée alors qu'une période antérieure ne l'est pas (AUDCIF art. 22 3°).
    #[error(
        "la période du {cloturee} est clôturée alors que celle du {anterieure}, antérieure, ne \
         l'est pas (AUDCIF art. 22 3°)"
    )]
    ClotureHorsOrdre {
        /// Premier jour de la période clôturée.
        cloturee: Date,
        /// Premier jour de la période antérieure non clôturée.
        anterieure: Date,
    },
    /// Dates hors de la plage représentable.
    #[error("date hors de la plage représentable")]
    DateHorsPlage,
}

type Erreur = ErreurReconstitutionExercices;

impl Exercices {
    /// Lignes plates des exercices et de leurs périodes, dans l'ordre chronologique : la forme
    /// stockée, relue par [`Exercices::reconstituer`].
    #[must_use]
    pub fn aplatir(&self) -> (Vec<LigneExercice>, Vec<LignePeriode>) {
        let exercices = self
            .exercices
            .iter()
            .map(|e| LigneExercice {
                code: e.code.clone(),
                debut: e.debut,
                fin: e.fin,
                statut: e.statut,
            })
            .collect();
        let periodes = self
            .exercices
            .iter()
            .flat_map(|e| {
                e.periodes.iter().map(|p| LignePeriode {
                    exercice: e.code.clone(),
                    debut: p.debut,
                    fin: p.fin,
                    statut: p.statut,
                })
            })
            .collect();
        (exercices, periodes)
    }

    /// Reconstitue les exercices **relus de la persistance** à partir de leurs lignes plates,
    /// dans un ordre quelconque.
    ///
    /// **Seul usage : relire la persistance.** Ce n'est pas une voie de transition : ouvrir,
    /// verrouiller ou clôturer passe par les fonctions de [`Exercices`]. Les contrôles sont
    /// structurels (voir le module) : ils garantissent que l'état relu est atteignable par les
    /// transitions, sans les rejouer. Le contexte d'alors (brouillard, échéances) n'est pas
    /// revérifié.
    ///
    /// # Errors
    /// [`ErreurReconstitutionExercices`] : les lignes ne décrivent pas un état atteignable.
    pub fn reconstituer(
        exercices: impl IntoIterator<Item = LigneExercice>,
        periodes: impl IntoIterator<Item = LignePeriode>,
    ) -> Result<Self, ErreurReconstitutionExercices> {
        let mut lignes: Vec<LigneExercice> = exercices.into_iter().collect();
        lignes.sort_by_key(|l| l.debut);
        let mut par_exercice: BTreeMap<CodeExercice, Vec<Periode>> = BTreeMap::new();
        for ligne in &lignes {
            if par_exercice
                .insert(ligne.code.clone(), Vec::new())
                .is_some()
            {
                return Err(Erreur::Doublon(ligne.code.clone()));
            }
        }
        for p in periodes {
            par_exercice
                .get_mut(&p.exercice)
                .ok_or_else(|| Erreur::PeriodeOrpheline {
                    exercice: p.exercice.clone(),
                    debut: p.debut,
                })?
                .push(Periode {
                    debut: p.debut,
                    fin: p.fin,
                    statut: p.statut,
                });
        }

        let mut reconstitues: Vec<Exercice> = Vec::with_capacity(lignes.len());
        for ligne in lignes {
            verifier_bornes(&ligne, reconstitues.last())?;
            let mut periodes = par_exercice.remove(&ligne.code).unwrap_or_default();
            periodes.sort_by_key(|p| p.debut);
            let attendu =
                decouper_en_mois(ligne.debut, ligne.fin).map_err(|_| Erreur::DateHorsPlage)?;
            let conforme = periodes.len() == attendu.len()
                && periodes
                    .iter()
                    .zip(&attendu)
                    .all(|(p, a)| (p.debut, p.fin) == (a.debut, a.fin));
            if !conforme {
                return Err(Erreur::DecoupageNonConforme(ligne.code));
            }
            reconstitues.push(Exercice {
                code: ligne.code,
                debut: ligne.debut,
                fin: ligne.fin,
                statut: ligne.statut,
                periodes,
            });
        }
        verifier_statuts(&reconstitues)?;
        Ok(Self {
            exercices: reconstitues,
        })
    }
}

/// Bornes d'un exercice relu, au regard de l'AUDCIF art. 7 et de l'exercice qui le précède.
fn verifier_bornes(ligne: &LigneExercice, precedent: Option<&Exercice>) -> Result<(), Erreur> {
    let (debut, fin) = (ligne.debut, ligne.fin);
    if (fin.month(), fin.day()) != (Month::December, 31) {
        return Err(Erreur::FinNonCivile {
            code: ligne.code.clone(),
            fin,
        });
    }
    let code = CodeExercice::de_l_annee(fin.year()).map_err(|_| Erreur::DateHorsPlage)?;
    if code != ligne.code {
        return Err(Erreur::CodeIncoherent {
            code: ligne.code.clone(),
            fin,
        });
    }
    let duree_conforme = match precedent {
        // Premier exercice : il se termine l'année de son début, ou l'année suivante s'il
        // débute au second semestre (à partir du 1er juillet).
        None => {
            fin.year() == debut.year()
                || (debut.year().checked_add(1) == Some(fin.year()) && debut.month() > Month::June)
        }
        // Exercices suivants : l'année civile qui suit la clôture du précédent.
        Some(precedent) => {
            if precedent.fin.next_day() != Some(debut) {
                return Err(Erreur::Discontinuite {
                    precedent: precedent.code.clone(),
                    suivant: ligne.code.clone(),
                });
            }
            fin.year() == debut.year()
        }
    };
    if duree_conforme {
        Ok(())
    } else {
        Err(Erreur::DureeNonConforme {
            code: ligne.code.clone(),
            debut,
            fin,
        })
    }
}

/// Avancement d'un statut d'exercice (ouvert → en clôture → clôturé).
fn rang(statut: StatutExercice) -> u8 {
    match statut {
        StatutExercice::Ouvert => 0,
        StatutExercice::EnCloture => 1,
        StatutExercice::Cloture => 2,
    }
}

/// Statuts relus : ceux qu'une suite de transitions peut produire.
fn verifier_statuts(exercices: &[Exercice]) -> Result<(), Erreur> {
    let non_clotures = exercices
        .iter()
        .filter(|e| e.statut != StatutExercice::Cloture)
        .count();
    if non_clotures > EXERCICES_NON_CLOTURES_MAX {
        return Err(Erreur::TropDExercicesNonClotures(non_clotures));
    }
    for paire in exercices.windows(2) {
        if rang(paire[1].statut) > rang(paire[0].statut) {
            return Err(Erreur::StatutsHorsOrdre {
                anterieur: paire[0].code.clone(),
                suivant: paire[1].code.clone(),
            });
        }
    }
    for exercice in exercices {
        if exercice.statut == StatutExercice::Cloture {
            if let Some(p) = exercice
                .periodes
                .iter()
                .find(|p| p.statut != StatutPeriode::Cloturee)
            {
                return Err(Erreur::PeriodeNonClotureeDansExerciceCloture {
                    exercice: exercice.code.clone(),
                    debut: p.debut,
                });
            }
        } else if exercice
            .periodes
            .last()
            .is_some_and(|p| p.statut == StatutPeriode::Cloturee)
        {
            return Err(Erreur::DernierePeriodeCloturee(exercice.code.clone()));
        }
    }
    let mut premiere_non_cloturee: Option<Date> = None;
    for p in exercices.iter().flat_map(|e| e.periodes.iter()) {
        match (p.statut, premiere_non_cloturee) {
            (StatutPeriode::Cloturee, Some(anterieure)) => {
                return Err(Erreur::ClotureHorsOrdre {
                    cloturee: p.debut,
                    anterieure,
                });
            }
            (StatutPeriode::Cloturee, None) | (_, Some(_)) => {}
            (_, None) => premiere_non_cloturee = Some(p.debut),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exercice::FinPremierExercice;
    use time::macros::date;

    fn code(texte: &str) -> CodeExercice {
        CodeExercice::parse(texte).unwrap()
    }

    /// Deux exercices : 2026 (premier, du 14 juillet, en clôture, juillet à novembre
    /// clôturés) et 2027 (ouvert).
    fn etat() -> Exercices {
        let mut exercices = Exercices::new();
        exercices
            .ouvrir_premier(date!(2026 - 07 - 14), FinPremierExercice::AnneeDeDebut)
            .unwrap();
        exercices.ouvrir_suivant().unwrap();
        exercices.passer_en_cloture(&code("2026")).unwrap();
        for mois in [7u8, 8, 9, 10, 11] {
            let jour = Date::from_calendar_date(2026, Month::try_from(mois).unwrap(), 20).unwrap();
            exercices.cloturer_periode(jour, []).unwrap();
        }
        exercices
            .verrouiller_periode(date!(2027 - 03 - 03))
            .unwrap();
        exercices
    }

    /// Reconstitue après avoir appliqué `modifier` aux lignes de [`etat`].
    fn relire(
        modifier: impl FnOnce(&mut Vec<LigneExercice>, &mut Vec<LignePeriode>),
    ) -> Result<Exercices, Erreur> {
        let (mut exercices, mut periodes) = etat().aplatir();
        modifier(&mut exercices, &mut periodes);
        Exercices::reconstituer(exercices, periodes)
    }

    fn rang_periode(periodes: &[LignePeriode], debut: Date) -> usize {
        periodes.iter().position(|p| p.debut == debut).unwrap()
    }

    #[test]
    fn aller_retour_dans_un_ordre_quelconque() {
        let etat = etat();
        let (mut exercices, mut periodes) = etat.aplatir();
        assert_eq!(exercices.len(), 2);
        assert_eq!(periodes.len(), 6 + 12);
        assert_eq!(
            Exercices::reconstituer(exercices.clone(), periodes.clone()),
            Ok(etat.clone())
        );
        exercices.reverse();
        periodes.reverse();
        periodes.swap(0, 7);
        assert_eq!(Exercices::reconstituer(exercices, periodes), Ok(etat));
        assert_eq!(Exercices::reconstituer([], []), Ok(Exercices::new()));
    }

    #[test]
    fn dossier_repris() {
        let mut exercices = Exercices::new();
        exercices.ouvrir_repris(2026).unwrap();
        let (e, p) = exercices.aplatir();
        assert_eq!(Exercices::reconstituer(e, p), Ok(exercices));
    }

    #[test]
    fn bornes_refusees() {
        assert_eq!(
            relire(|e, _| e[0].code = code("2027")),
            Err(Erreur::Doublon(code("2027")))
        );
        assert_eq!(
            relire(|e, p| {
                e[1].code = code("2028");
                p.retain(|p| p.exercice != code("2027"));
            }),
            Err(Erreur::CodeIncoherent {
                code: code("2028"),
                fin: date!(2027 - 12 - 31),
            })
        );
        assert_eq!(
            relire(|e, _| e[1].fin = date!(2027 - 11 - 30)),
            Err(Erreur::FinNonCivile {
                code: code("2027"),
                fin: date!(2027 - 11 - 30),
            })
        );
        // Premier exercice de plus d'un an débutant au premier semestre.
        assert_eq!(
            relire(|e, p| {
                e[0].debut = date!(2025 - 06 - 30);
                e.truncate(1);
                p.retain(|p| p.exercice == code("2026"));
            }),
            Err(Erreur::DureeNonConforme {
                code: code("2026"),
                debut: date!(2025 - 06 - 30),
                fin: date!(2026 - 12 - 31),
            })
        );
        // Premier exercice de plus de deux années civiles.
        assert!(matches!(
            relire(|e, p| {
                e[0].debut = date!(2024 - 07 - 14);
                e.truncate(1);
                p.retain(|p| p.exercice == code("2026"));
            }),
            Err(Erreur::DureeNonConforme { .. })
        ));
        // Exercice suivant sur deux années civiles : 2028 qui commencerait en 2027.
        assert!(matches!(
            relire(|e, p| {
                e[1].code = code("2028");
                e[1].fin = date!(2028 - 12 - 31);
                for p in p.iter_mut().filter(|p| p.exercice == code("2027")) {
                    p.exercice = code("2028");
                }
            }),
            Err(Erreur::DureeNonConforme { .. })
        ));
        // Trou entre deux exercices.
        assert_eq!(
            relire(|e, _| e[1].debut = date!(2027 - 01 - 02)),
            Err(Erreur::Discontinuite {
                precedent: code("2026"),
                suivant: code("2027"),
            })
        );
        assert_eq!(
            relire(|_, p| p[0].exercice = code("2030")),
            Err(Erreur::PeriodeOrpheline {
                exercice: code("2030"),
                debut: date!(2026 - 07 - 14),
            })
        );
    }

    #[test]
    fn decoupage_refuse() {
        let attendu = Err(Erreur::DecoupageNonConforme(code("2027")));
        // Période manquante, en double, aux bornes déplacées, ou rattachée au mauvais
        // exercice.
        assert_eq!(
            relire(|_, p| {
                p.pop();
            }),
            attendu
        );
        assert_eq!(
            relire(|_, p| {
                let derniere = p[p.len() - 1].clone();
                p.push(derniere);
            }),
            attendu
        );
        assert_eq!(
            relire(|_, p| {
                let i = rang_periode(p, date!(2027 - 02 - 01));
                p[i].fin = date!(2027 - 02 - 27);
            }),
            attendu
        );
        assert_eq!(
            relire(|_, p| {
                let i = rang_periode(p, date!(2026 - 12 - 01));
                p[i].exercice = code("2027");
            }),
            Err(Erreur::DecoupageNonConforme(code("2026")))
        );
        // Premier mois amputé : le premier exercice commence le 14 juillet.
        assert_eq!(
            relire(|_, p| p[0].debut = date!(2026 - 07 - 01)),
            Err(Erreur::DecoupageNonConforme(code("2026")))
        );
    }

    #[test]
    fn statuts_refuses() {
        assert_eq!(
            relire(|e, _| e[1].statut = StatutExercice::Cloture),
            Err(Erreur::StatutsHorsOrdre {
                anterieur: code("2026"),
                suivant: code("2027"),
            })
        );
        assert_eq!(
            relire(|e, _| e[0].statut = StatutExercice::Cloture),
            Err(Erreur::PeriodeNonClotureeDansExerciceCloture {
                exercice: code("2026"),
                debut: date!(2026 - 12 - 01),
            })
        );
        assert_eq!(
            relire(|_, p| {
                let i = rang_periode(p, date!(2026 - 12 - 01));
                p[i].statut = StatutPeriode::Cloturee;
            }),
            Err(Erreur::DernierePeriodeCloturee(code("2026")))
        );
        assert_eq!(
            relire(|_, p| {
                let i = rang_periode(p, date!(2026 - 09 - 01));
                p[i].statut = StatutPeriode::Verrouillee;
            }),
            Err(Erreur::ClotureHorsOrdre {
                cloturee: date!(2026 - 10 - 01),
                anterieure: date!(2026 - 09 - 01),
            })
        );
        // Une période de 2027 clôturée alors que décembre 2026 ne l'est pas.
        assert_eq!(
            relire(|_, p| {
                let i = rang_periode(p, date!(2027 - 01 - 01));
                p[i].statut = StatutPeriode::Cloturee;
            }),
            Err(Erreur::ClotureHorsOrdre {
                cloturee: date!(2027 - 01 - 01),
                anterieure: date!(2026 - 12 - 01),
            })
        );
        // Trois exercices non clôturés.
        let mut exercices = Exercices::new();
        exercices.ouvrir_repris(2026).unwrap();
        exercices.ouvrir_suivant().unwrap();
        let (mut e, mut p) = exercices.aplatir();
        let mut troisieme = Exercices::new();
        troisieme.ouvrir_repris(2028).unwrap();
        let (e3, p3) = troisieme.aplatir();
        e.extend(e3);
        p.extend(p3);
        assert_eq!(
            Exercices::reconstituer(e, p),
            Err(Erreur::TropDExercicesNonClotures(3))
        );
    }
}
