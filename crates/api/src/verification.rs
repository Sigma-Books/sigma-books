//! Vérification de la chaîne d'un dossier **depuis la base** (L2-31, PR 15b, #188 ; ADR 0007,
//! 0010, 0020 §4 ; menaces F2.2, F2.4, F2.5).
//!
//! Module sans entrée-sortie : il reçoit les écritures validées dans l'ordre de leur rang
//! ([`LigneChaine`], lues par [`crate::base::integrite`]) et les jetons de la base
//! ([`JetonStocke`]), et rend un [`Rapport`]. Il détecte ce que le rôle applicatif ne peut pas
//! faire mais que le propriétaire des tables, une sauvegarde restaurée ou une réécriture
//! d'une partie de la base peuvent faire :
//! - texte canonique modifié (empreinte recalculée différente de l'empreinte stockée) ;
//! - empreinte précédente, rang ou colonnes qui ne concordent pas avec le texte haché
//!   (numéro, dates, libellé, format, lignes) ;
//! - tête de chaîne stockée différente de la tête recalculée (retour en arrière, troncature) ;
//! - jeton dont le témoin (nombre, tête) n'est pas celui de la chaîne recalculée, ou dont la
//!   signature, la chaîne de certificats ou l'empreinte ne se vérifient plus.
//!
//! Après une anomalie, le calcul **reprend sur l'empreinte stockée** : seule l'écriture altérée est
//! signalée, pas toutes les suivantes. La première anomalie localise la première écriture
//! altérée.
//!
//! Les rapports ne citent que des codes stables, des rangs, des numéros et le nom d'une colonne :
//! jamais un libellé, un montant ni le texte d'une réponse d'autorité.

use std::collections::BTreeMap;

use integrite::horodatage::{Ancre, Temoin};
use integrite::verification::ANOMALIES_DETAILLEES;
use integrite::{EcritureCanonique, Empreinte, IdDossier, chainer, genese};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

/// Une écriture validée telle que la base la porte : texte haché et colonnes à rapprocher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LigneChaine {
    /// Rang dans la chaîne du dossier.
    pub rang: i64,
    /// Numéro (colonne).
    pub numero: String,
    /// Date comptable (colonne).
    pub date: Date,
    /// Date de validation (colonne).
    pub date_validation: Date,
    /// Date de valeur comptable (colonne).
    pub date_valeur_comptable: Option<Date>,
    /// Libellé (colonne).
    pub libelle: String,
    /// Version du format canonique (colonne).
    pub format: i16,
    /// Contenu canonique haché.
    pub contenu: String,
    /// Empreinte stockée.
    pub empreinte: Empreinte,
    /// Empreinte précédente stockée.
    pub precedente: Empreinte,
    /// Lignes (colonnes), au format `compte:D|C:montant` (montant canonique).
    pub lignes: Vec<String>,
}

/// Jeton d'horodatage stocké.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetonStocke {
    /// Identifiant.
    pub id: Uuid,
    /// Nombre d'écritures couvertes.
    pub nombre: i64,
    /// Tête de chaîne horodatée.
    pub tete: Empreinte,
    /// Autorité.
    pub autorite: String,
    /// Date attestée.
    pub date_jeton: OffsetDateTime,
    /// Instant d'obtention.
    pub obtenu_le: OffsetDateTime,
    /// Déclencheur.
    pub declencheur: String,
    /// Jeton RFC 3161 (DER).
    pub der: Vec<u8>,
}

/// Contrôle d'un jeton contre son témoin : renvoie un code stable en cas de refus.
pub trait ControleJeton {
    /// Vérifie le jeton (empreinte du témoin, signature, chaîne de certificats, ancre).
    ///
    /// # Errors
    /// Code stable du refus.
    fn verifier(&self, der: &[u8], temoin: &Temoin) -> Result<(), &'static str>;
}

/// Contrôle par `integrite` contre les ancres de confiance.
#[derive(Debug)]
pub struct ControleOfficiel {
    ancres: Vec<Ancre>,
}

impl ControleOfficiel {
    /// Contrôle contre ces ancres.
    #[must_use]
    pub fn new(ancres: Vec<Ancre>) -> Self {
        Self { ancres }
    }
}

impl ControleJeton for ControleOfficiel {
    fn verifier(&self, der: &[u8], temoin: &Temoin) -> Result<(), &'static str> {
        integrite::verifier_jeton(der, temoin, &self.ancres)
            .map(|_| ())
            .map_err(|_| "jeton_refuse")
    }
}

/// Anomalie constatée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomalie {
    /// Code stable.
    pub code: &'static str,
    /// Rang de l'écriture (ou nombre d'écritures du jeton) concernée.
    pub rang: Option<i64>,
    /// Numéro de l'écriture concernée.
    pub numero: Option<String>,
    /// Précision : colonne en cause, ou motif du refus d'un jeton.
    pub detail: Option<&'static str>,
}

/// Résultat de la vérification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rapport {
    /// Écritures lues.
    pub nombre: u64,
    /// Tête recalculée.
    pub tete: Empreinte,
    /// Jetons examinés.
    pub jetons: u64,
    /// Jetons valides (témoin concordant et signature vérifiée).
    pub jetons_valides: u64,
    /// Jetons dont le témoin concorde mais dont la signature n'a pas été vérifiée, faute de
    /// place dans la borne de signatures par vérification : les plus anciens.
    pub jetons_non_verifies: u64,
    /// Anomalies détaillées (au plus [`ANOMALIES_DETAILLEES`]).
    pub anomalies: Vec<Anomalie>,
    /// Anomalies non détaillées.
    pub anomalies_omises: u64,
}

impl Rapport {
    /// Vrai si aucune anomalie n'a été constatée.
    #[must_use]
    pub fn est_integre(&self) -> bool {
        self.anomalies.is_empty() && self.anomalies_omises == 0
    }
}

/// Vérificateur de chaîne, alimenté écriture par écriture.
#[derive(Debug)]
pub struct VerificateurChaine {
    courante: Empreinte,
    rang_attendu: i64,
    lues: u64,
    dernier_rang: i64,
    tetes: BTreeMap<i64, Option<Empreinte>>,
    anomalies: Vec<Anomalie>,
    omises: u64,
}

/// Numéro de format lu sur la première ligne d'un contenu canonique.
fn format_du_texte(contenu: &str) -> Option<i16> {
    match contenu.lines().next()? {
        "ecriture v1" => Some(1),
        "ecriture v2" => Some(2),
        "ecriture v3" => Some(3),
        "ecriture v4" => Some(4),
        _ => None,
    }
}

/// Valeur de la première ligne `cle=…` de l'en-tête du contenu (avant les pièces et les lignes).
fn valeur<'a>(contenu: &'a str, cle: &str) -> Option<&'a str> {
    contenu
        .lines()
        .take_while(|l| *l != "ligne")
        .find_map(|l| l.strip_prefix(cle)?.strip_prefix('='))
}

/// Lignes du contenu, au format `compte:D|C:montant`, triées.
fn lignes_du_texte(contenu: &str) -> Vec<String> {
    let mut lignes: Vec<String> = Vec::new();
    let mut courante: Option<(String, String, String)> = None;
    let fermer = |c: &mut Option<(String, String, String)>, sortie: &mut Vec<String>| {
        if let Some((compte, sens, montant)) = c.take() {
            sortie.push(format!("{compte}:{sens}:{montant}"));
        }
    };
    for l in contenu.lines() {
        if l == "ligne" {
            fermer(&mut courante, &mut lignes);
            courante = Some(Default::default());
        } else if let Some(c) = courante.as_mut() {
            if let Some(v) = l.strip_prefix("compte=") {
                v.clone_into(&mut c.0);
            } else if let Some(v) = l.strip_prefix("sens=") {
                v.clone_into(&mut c.1);
            } else if let Some(v) = l.strip_prefix("montant=") {
                v.clone_into(&mut c.2);
            }
        }
    }
    fermer(&mut courante, &mut lignes);
    lignes.sort();
    lignes
}

fn date_iso(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

impl VerificateurChaine {
    /// Vérificateur d'une chaîne de ce dossier ; les têtes des rangs `rangs` seront conservées
    /// pour contrôler les témoins des jetons.
    #[must_use]
    pub fn new(dossier: &IdDossier, rangs: impl IntoIterator<Item = i64>) -> Self {
        Self {
            courante: genese(dossier),
            rang_attendu: 1,
            lues: 0,
            dernier_rang: 0,
            tetes: rangs.into_iter().map(|r| (r, None)).collect(),
            anomalies: Vec::new(),
            omises: 0,
        }
    }

    fn signaler(&mut self, code: &'static str, l: &LigneChaine, detail: Option<&'static str>) {
        if self.anomalies.len() < ANOMALIES_DETAILLEES {
            self.anomalies.push(Anomalie {
                code,
                rang: Some(l.rang),
                numero: Some(l.numero.clone()),
                detail,
            });
        } else {
            self.omises += 1;
        }
    }

    /// Écritures lues jusqu'ici.
    #[must_use]
    pub fn lues(&self) -> u64 {
        self.lues
    }

    /// Ajoute l'écriture suivante (dans l'ordre des rangs).
    pub fn ajouter(&mut self, l: &LigneChaine) {
        if l.rang != self.rang_attendu {
            self.signaler("rang_discontinu", l, None);
        }
        self.rang_attendu = l.rang.saturating_add(1);
        if l.precedente != self.courante {
            self.signaler("precedente_incorrecte", l, None);
        }
        if chainer(&self.courante, l.contenu.as_bytes()) != l.empreinte {
            self.signaler("empreinte_alteree", l, None);
        }
        self.controler_colonnes(l);
        // Reprise sur l'empreinte stockée : l'écriture altérée est seule signalée.
        self.courante = l.empreinte;
        self.lues += 1;
        self.dernier_rang = l.rang;
        if let Some(tete) = self.tetes.get_mut(&l.rang) {
            *tete = Some(l.empreinte);
        }
    }

    fn controler_colonnes(&mut self, l: &LigneChaine) {
        let Ok(ecriture) = EcritureCanonique::lire(&l.contenu) else {
            self.signaler("contenu_illisible", l, None);
            return;
        };
        let mut ecarts: Vec<&'static str> = Vec::new();
        if format_du_texte(&l.contenu) != Some(l.format) {
            ecarts.push("format");
        }
        if ecriture.numero().to_string() != l.numero
            || valeur(&l.contenu, "numero") != Some(l.numero.as_str())
        {
            ecarts.push("numero");
        }
        if valeur(&l.contenu, "date") != Some(date_iso(l.date).as_str()) {
            ecarts.push("date");
        }
        if valeur(&l.contenu, "date-validation") != Some(date_iso(l.date_validation).as_str()) {
            ecarts.push("date_validation");
        }
        if valeur(&l.contenu, "date-valeur-comptable")
            != l.date_valeur_comptable.map(date_iso).as_deref()
        {
            ecarts.push("date_valeur_comptable");
        }
        if valeur(&l.contenu, "libelle") != Some(l.libelle.as_str()) {
            ecarts.push("libelle");
        }
        let mut en_base = l.lignes.clone();
        en_base.sort();
        if en_base != lignes_du_texte(&l.contenu) {
            ecarts.push("lignes");
        }
        for colonne in ecarts {
            self.signaler("colonne_differente", l, Some(colonne));
        }
    }

    /// Termine : compare la tête recalculée à la tête stockée (`stockee` : rang et empreinte de
    /// `chaine_tete`, absente si aucune écriture n'a été validée) et contrôle les jetons : le
    /// témoin de chacun, la signature des `signatures_max` plus récents seulement (une signature
    /// RSA coûte du calcul ; les jetons sont rendus du plus ancien au plus récent).
    #[must_use]
    pub fn terminer(
        mut self,
        dossier: &IdDossier,
        stockee: Option<(i64, Empreinte)>,
        jetons: &[JetonStocke],
        controle: &dyn ControleJeton,
        signatures_max: usize,
    ) -> Rapport {
        let signaler = |anomalies: &mut Vec<Anomalie>,
                        omises: &mut u64,
                        code: &'static str,
                        rang: Option<i64>,
                        detail: Option<&'static str>| {
            if anomalies.len() < ANOMALIES_DETAILLEES {
                anomalies.push(Anomalie {
                    code,
                    rang,
                    numero: None,
                    detail,
                });
            } else {
                *omises += 1;
            }
        };
        let (anomalies, omises) = (&mut self.anomalies, &mut self.omises);
        match stockee {
            Some((rang, tete)) => {
                if rang != self.dernier_rang {
                    signaler(anomalies, omises, "tete_rang_different", Some(rang), None);
                } else if tete != self.courante {
                    signaler(anomalies, omises, "tete_differente", Some(rang), None);
                }
            }
            None if self.lues > 0 => {
                signaler(anomalies, omises, "tete_absente", None, None);
            }
            None => {}
        }
        let mut valides = 0_u64;
        let mut non_verifies = 0_u64;
        let premiere_signature = jetons.len().saturating_sub(signatures_max);
        for (i, jeton) in jetons.iter().enumerate() {
            let tete_au_rang = if jeton.nombre == 0 {
                Some(genese(dossier))
            } else {
                self.tetes.get(&jeton.nombre).copied().flatten()
            };
            let rang = Some(jeton.nombre);
            match tete_au_rang {
                None => signaler(anomalies, omises, "jeton_hors_chaine", rang, Some("nombre")),
                Some(t) if t != jeton.tete => {
                    signaler(anomalies, omises, "jeton_hors_chaine", rang, Some("tete"));
                }
                Some(_) if i < premiere_signature => non_verifies += 1,
                Some(_) => {
                    let temoin = Temoin::nouveau(
                        dossier.clone(),
                        jeton.tete,
                        u64::try_from(jeton.nombre).unwrap_or(0),
                    );
                    match controle.verifier(&jeton.der, &temoin) {
                        Ok(()) => valides += 1,
                        Err(motif) => {
                            signaler(anomalies, omises, "jeton_invalide", rang, Some(motif));
                        }
                    }
                }
            }
        }
        Rapport {
            nombre: self.lues,
            tete: self.courante,
            jetons: jetons.len() as u64,
            jetons_valides: valides,
            jetons_non_verifies: non_verifies,
            anomalies: self.anomalies,
            anomalies_omises: self.omises,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use integrite::export::{Element, LecteurExport};
    use proptest::prelude::*;

    const EXPORT: &str = include_str!("../../integrite/tests/vecteurs/export-v3-reference.txt");

    /// Écritures du vecteur figé, avec des colonnes cohérentes avec leur texte.
    fn vecteur() -> (IdDossier, Vec<LigneChaine>) {
        let (mut lecteur, dossier) = LecteurExport::ouvrir(EXPORT.as_bytes()).unwrap();
        let mut lignes = Vec::new();
        let mut precedente = genese(&dossier);
        let mut rang = 0;
        while let Some(e) = lecteur.suivant().unwrap() {
            if let Element::Ecriture {
                contenu, empreinte, ..
            } = e
            {
                rang += 1;
                lignes.push(LigneChaine {
                    rang,
                    numero: valeur(&contenu, "numero").unwrap().to_owned(),
                    date: date(valeur(&contenu, "date").unwrap()),
                    date_validation: date(valeur(&contenu, "date-validation").unwrap()),
                    date_valeur_comptable: valeur(&contenu, "date-valeur-comptable").map(date),
                    libelle: valeur(&contenu, "libelle").unwrap().to_owned(),
                    format: format_du_texte(&contenu).unwrap(),
                    lignes: lignes_du_texte(&contenu),
                    contenu,
                    empreinte,
                    precedente,
                });
                precedente = empreinte;
            }
        }
        (dossier, lignes)
    }

    fn date(texte: &str) -> Date {
        Date::parse(
            texte,
            &time::macros::format_description!("[year]-[month]-[day]"),
        )
        .unwrap()
    }

    struct Accepte;
    impl ControleJeton for Accepte {
        fn verifier(&self, _: &[u8], _: &Temoin) -> Result<(), &'static str> {
            Ok(())
        }
    }
    struct Refuse;
    impl ControleJeton for Refuse {
        fn verifier(&self, _: &[u8], _: &Temoin) -> Result<(), &'static str> {
            Err("jeton_refuse")
        }
    }

    fn verifier(
        dossier: &IdDossier,
        lignes: &[LigneChaine],
        stockee: Option<(i64, Empreinte)>,
        jetons: &[JetonStocke],
        controle: &dyn ControleJeton,
    ) -> Rapport {
        let rangs = jetons.iter().map(|j| j.nombre);
        let mut v = VerificateurChaine::new(dossier, rangs);
        for l in lignes {
            v.ajouter(l);
        }
        v.terminer(dossier, stockee, jetons, controle, 1000)
    }

    fn jeton(nombre: i64, tete: Empreinte) -> JetonStocke {
        JetonStocke {
            id: Uuid::nil(),
            nombre,
            tete,
            autorite: "tsa".into(),
            date_jeton: OffsetDateTime::UNIX_EPOCH,
            obtenu_le: OffsetDateTime::UNIX_EPOCH,
            declencheur: "quotidien".into(),
            der: vec![1],
        }
    }

    #[test]
    fn le_vecteur_est_integre_et_sa_tete_concorde() {
        let (d, lignes) = vecteur();
        assert!(lignes.len() >= 3);
        let tete = lignes.last().unwrap().empreinte;
        let n = i64::try_from(lignes.len()).unwrap();
        let r = verifier(&d, &lignes, Some((n, tete)), &[], &Accepte);
        assert!(r.est_integre(), "{:?}", r.anomalies);
        assert_eq!((r.nombre, r.tete), (lignes.len() as u64, tete));
    }

    #[test]
    fn une_chaine_vide_est_integre_sans_tete_stockee() {
        let (d, _) = vecteur();
        let r = verifier(&d, &[], None, &[], &Accepte);
        assert!(r.est_integre());
        assert_eq!(r.tete, genese(&d));
    }

    #[test]
    fn un_texte_modifie_localise_la_premiere_ecriture_alteree() {
        let (d, mut lignes) = vecteur();
        let tete = lignes.last().unwrap().empreinte;
        let n = i64::try_from(lignes.len()).unwrap();
        lignes[1].contenu = lignes[1].contenu.replace("libelle=", "libelle=X");
        let r = verifier(&d, &lignes, Some((n, tete)), &[], &Accepte);
        assert!(!r.est_integre());
        let premiere = &r.anomalies[0];
        assert_eq!(
            (premiere.code, premiere.rang),
            ("empreinte_alteree", Some(2))
        );
        // Les suivantes ne sont pas signalées en cascade : le calcul reprend sur l'empreinte
        // stockée, et la tête reste celle du stockage.
        assert!(
            r.anomalies.iter().all(|a| a.rang == Some(2)),
            "{:?}",
            r.anomalies
        );
    }

    #[test]
    fn une_colonne_modifiee_est_nommee() {
        let (d, mut lignes) = vecteur();
        let tete = lignes.last().unwrap().empreinte;
        let n = i64::try_from(lignes.len()).unwrap();
        lignes[0].libelle.push('!');
        lignes[2].lignes[0] = "999:D:1".to_owned();
        let r = verifier(&d, &lignes, Some((n, tete)), &[], &Accepte);
        let codes: Vec<_> = r
            .anomalies
            .iter()
            .map(|a| (a.code, a.rang, a.detail))
            .collect();
        assert_eq!(
            codes,
            [
                ("colonne_differente", Some(1), Some("libelle")),
                ("colonne_differente", Some(3), Some("lignes")),
            ]
        );
    }

    #[test]
    fn rang_precedente_et_tete_stockee_sont_controles() {
        let (d, lignes) = vecteur();
        let tete = lignes.last().unwrap().empreinte;
        let n = i64::try_from(lignes.len()).unwrap();
        // Écriture supprimée au milieu.
        let mut trous = lignes.clone();
        trous.remove(1);
        let r = verifier(&d, &trous, Some((n, tete)), &[], &Accepte);
        let codes: Vec<_> = r.anomalies.iter().map(|a| a.code).collect();
        assert!(codes.contains(&"rang_discontinu") && codes.contains(&"precedente_incorrecte"));
        // Chaîne tronquée (dernière écriture supprimée) face à la tête stockée.
        let r = verifier(
            &d,
            &lignes[..lignes.len() - 1],
            Some((n, tete)),
            &[],
            &Accepte,
        );
        assert_eq!(r.anomalies[0].code, "tete_rang_different");
        // Tête stockée retouchée.
        let autre = chainer(&tete, b"x");
        let r = verifier(&d, &lignes, Some((n, autre)), &[], &Accepte);
        assert_eq!(r.anomalies[0].code, "tete_differente");
        // Chaîne vidée alors qu'une tête existait.
        let r = verifier(&d, &[], Some((n, tete)), &[], &Accepte);
        assert_eq!(r.anomalies[0].code, "tete_rang_different");
        let r = verifier(&d, &lignes, None, &[], &Accepte);
        assert_eq!(r.anomalies[0].code, "tete_absente");
    }

    #[test]
    fn les_jetons_sont_rapproches_de_la_chaine_recalculee() {
        let (d, lignes) = vecteur();
        let tete = lignes.last().unwrap().empreinte;
        let n = i64::try_from(lignes.len()).unwrap();
        let t2 = lignes[1].empreinte;
        let bons = [jeton(2, t2), jeton(n, tete)];
        let r = verifier(&d, &lignes, Some((n, tete)), &bons, &Accepte);
        assert!(r.est_integre());
        assert_eq!((r.jetons, r.jetons_valides), (2, 2));
        // Témoin d'une autre tête (chaîne réécrite depuis le jeton), ou d'un rang inexistant.
        let faux = [jeton(2, tete), jeton(n + 5, tete)];
        let r = verifier(&d, &lignes, Some((n, tete)), &faux, &Accepte);
        let a: Vec<_> = r.anomalies.iter().map(|a| (a.code, a.detail)).collect();
        assert_eq!(
            a,
            [
                ("jeton_hors_chaine", Some("tete")),
                ("jeton_hors_chaine", Some("nombre"))
            ]
        );
        // Signature refusée.
        let r = verifier(&d, &lignes, Some((n, tete)), &bons, &Refuse);
        assert_eq!(r.jetons_valides, 0);
        assert_eq!(r.anomalies[0].code, "jeton_invalide");
    }

    #[test]
    fn seules_les_signatures_les_plus_recentes_sont_verifiees() {
        let (d, lignes) = vecteur();
        let tete = lignes.last().unwrap().empreinte;
        let n = i64::try_from(lignes.len()).unwrap();
        let jetons = [
            jeton(1, lignes[0].empreinte),
            jeton(2, lignes[1].empreinte),
            jeton(n, tete),
        ];
        let mut v = VerificateurChaine::new(&d, jetons.iter().map(|j| j.nombre));
        for l in &lignes {
            v.ajouter(l);
        }
        let r = v.terminer(&d, Some((n, tete)), &jetons, &Accepte, 1);
        assert!(r.est_integre());
        assert_eq!(
            (r.jetons, r.jetons_valides, r.jetons_non_verifies),
            (3, 1, 2)
        );
    }

    #[test]
    fn le_jeton_figé_de_l_autorite_se_verifie_avec_les_ancres_officielles() {
        // Vecteur de L1 : témoin du dossier « demo », tête TETE, aucune écriture.
        let der = include_bytes!("../../integrite/tests/vecteurs/horodatage/digicert.tsr");
        let temoin = Temoin::nouveau(
            IdDossier::parse("demo").unwrap(),
            Empreinte::parse("ea5595d5652ca52169b5d87e9bfbcae14eafc1031c7e2f7985af5252dff7229c")
                .unwrap(),
            0,
        );
        let controle = ControleOfficiel::new(integrite::ancres_par_defaut().unwrap());
        // Le fichier est une réponse (`TimeStampResp`), pas un jeton : refusé, par un code stable.
        assert_eq!(controle.verifier(der, &temoin), Err("jeton_refuse"));
    }

    proptest! {
        /// Toute altération d'un octet du texte d'une écriture est localisée à son rang.
        #[test]
        fn un_octet_modifie_est_toujours_detecte(ecriture in 0_usize..3, position in 0_usize..400) {
            let (d, mut lignes) = vecteur();
            let tete = lignes.last().unwrap().empreinte;
            let n = i64::try_from(lignes.len()).unwrap();
            let i = ecriture % lignes.len();
            let mut octets = lignes[i].contenu.clone().into_bytes();
            let p = position % octets.len();
            octets[p] = if octets[p] == b'x' { b'y' } else { b'x' };
            let Ok(modifie) = String::from_utf8(octets) else { return Ok(()); };
            lignes[i].contenu = modifie;
            let r = verifier(&d, &lignes, Some((n, tete)), &[], &Accepte);
            prop_assert!(!r.est_integre());
            prop_assert_eq!(r.anomalies[0].rang, Some(i64::try_from(i).unwrap() + 1));
            prop_assert_eq!(r.anomalies[0].code, "empreinte_alteree");
        }
    }
}
