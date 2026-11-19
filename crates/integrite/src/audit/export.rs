//! Export d'audit d'une portée (ADR 0014, « Rôles et lecture » ; `docs/integrite/audit-v1.md`
//! §5) : les entrées dans l'ordre de la chaîne, chacune suivie de son empreinte, puis la tête,
//! le nombre d'entrées et une **annexe non chaînée** qui associe chaque UUID d'acteur à son nom
//! au moment de l'export.
//!
//! La lecture traite le fichier comme **hostile** : lecture ligne à ligne bornée (1 024 octets
//! par ligne, 4 Mio par entrée), erreurs typées avec numéro de ligne, aucune panique. Une entrée
//! illisible n'arrête pas la vérification : elle est signalée et chaînée telle quelle.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Read as _, Write};

use crate::chaine::Empreinte;
use crate::export::{ErreurExport, ErreurLecture, FIN_EXPORT, LIGNE_MAX, lire_nombre};

use super::chaine::{RapportAudit, Verificateur};
use super::entree::{self, Acteur, ENTREE_MAX, EntreeAudit, Portee, Texte, Uuid};

/// Première ligne d'un export d'audit.
pub const ENTETE_EXPORT_AUDIT: &str = "sigma-books export-audit v1";
/// Début de l'annexe des acteurs.
pub const ANNEXE: &str = "annexe-acteurs";
/// Fin de l'annexe des acteurs.
pub const FIN_ANNEXE: &str = "fin-annexe";

/// Écrit un export d'audit, entrée par entrée.
pub struct EcrivainExportAudit<W: Write> {
    sortie: W,
    portee: Portee,
    nombre: u64,
    acteurs: BTreeSet<Uuid>,
}

impl<W: Write> EcrivainExportAudit<W> {
    /// Écrit l'en-tête de l'export de la chaîne de `portee`.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn nouveau(portee: Portee, mut sortie: W) -> io::Result<Self> {
        writeln!(sortie, "{ENTETE_EXPORT_AUDIT}")?;
        writeln!(sortie, "portee={}", portee.nom())?;
        writeln!(sortie, "cabinet={}", portee.cabinet())?;
        if let Some(dossier) = portee.dossier() {
            writeln!(sortie, "dossier={dossier}")?;
        }
        Ok(Self {
            sortie,
            portee,
            nombre: 0,
            acteurs: BTreeSet::new(),
        })
    }

    /// Ajoute une entrée enregistrée, telle quelle, et l'empreinte enregistrée pour elle : la
    /// vérification de l'export dira si elles sont intègres. L'entrée doit être une entrée
    /// canonique de la portée (sinon le texte pourrait altérer la structure de l'export).
    ///
    /// # Errors
    /// Entrée illisible ou d'une autre portée (`InvalidData`), erreur d'écriture.
    pub fn entree(&mut self, contenu: &str, empreinte: &Empreinte) -> io::Result<()> {
        let entree = EntreeAudit::lire(contenu)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if *entree.portee() != self.portee {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "entrée d'une autre portée",
            ));
        }
        if let Acteur::Utilisateur { id, .. } = entree.evenement().acteur() {
            self.acteurs.insert(*id);
        }
        self.sortie.write_all(contenu.as_bytes())?;
        writeln!(self.sortie, "empreinte={empreinte}")?;
        self.nombre += 1;
        Ok(())
    }

    /// UUID des acteurs des entrées écrites, dans l'ordre : ceux dont l'annexe donne le nom.
    #[must_use]
    pub fn acteurs(&self) -> &BTreeSet<Uuid> {
        &self.acteurs
    }

    /// Écrit la tête enregistrée, le nombre d'entrées écrites et l'annexe : chaque acteur des
    /// entrées, avec son nom s'il figure dans `noms`. Rend la sortie et le nombre d'entrées.
    ///
    /// # Errors
    /// Erreur d'écriture.
    pub fn terminer(
        mut self,
        tete: &Empreinte,
        noms: &BTreeMap<Uuid, Texte>,
    ) -> io::Result<(W, u64)> {
        writeln!(self.sortie, "tete={tete}")?;
        writeln!(self.sortie, "nombre={}", self.nombre)?;
        writeln!(self.sortie, "{ANNEXE}")?;
        for acteur in &self.acteurs {
            writeln!(self.sortie, "acteur={acteur}")?;
            if let Some(nom) = noms.get(acteur) {
                writeln!(self.sortie, "nom={nom}")?;
            }
        }
        writeln!(self.sortie, "{FIN_ANNEXE}")?;
        writeln!(self.sortie, "{FIN_EXPORT}")?;
        self.sortie.flush()?;
        Ok((self.sortie, self.nombre))
    }
}

/// Ce que le vérificateur attend d'un export d'audit, connu par ailleurs : tête (témoin
/// externe), cabinet, dossier. Chaque attente absente n'est pas contrôlée.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttentesAudit {
    /// Empreinte de tête connue par ailleurs (`--tete`).
    pub tete: Option<Empreinte>,
    /// Cabinet attendu (`--cabinet`).
    pub cabinet: Option<Uuid>,
    /// Dossier attendu (`--dossier`) : l'export doit être celui de ce dossier.
    pub dossier: Option<Uuid>,
}

/// Résultat de la vérification d'un export d'audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RapportExportAudit {
    /// Vérification de la chaîne (forme des entrées, empreintes, rangs, tête, nombre).
    pub chaine: RapportAudit,
    /// Annexe des acteurs, non chaînée : UUID et nom au moment de l'export.
    pub annexe: Vec<(Uuid, Option<Texte>)>,
    /// Faux si l'annexe ne cite pas exactement les acteurs des entrées.
    pub annexe_coherente: bool,
    /// Ancrage externe fourni (`--tete`) et différent de la tête recalculée.
    pub ancrage_different: bool,
    /// Cabinet ou dossier attendu (`--cabinet`, `--dossier`) différent de celui de l'export.
    pub portee_inattendue: bool,
}

impl RapportExportAudit {
    /// Vrai si la chaîne est intègre, l'annexe cohérente et l'ancrage, s'il est fourni, égal.
    #[must_use]
    pub fn est_integre(&self) -> bool {
        self.chaine.est_integre()
            && self.annexe_coherente
            && !self.ancrage_different
            && !self.portee_inattendue
    }
}

impl std::fmt::Display for RapportExportAudit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.chaine)?;
        if !self.annexe_coherente {
            writeln!(
                f,
                "Anomalie : l'annexe ne cite pas exactement les acteurs des entrées."
            )?;
        }
        if self.ancrage_different {
            writeln!(
                f,
                "Anomalie : tête différente de l'ancrage fourni (--tete)."
            )?;
        }
        if self.portee_inattendue {
            writeln!(
                f,
                "Anomalie : export d'un autre cabinet ou d'un autre dossier que celui attendu \
                 (--cabinet, --dossier)."
            )?;
        }
        writeln!(
            f,
            "Annexe (non chaînée, noms au moment de l'export) : {} acteur(s).",
            self.annexe.len()
        )
    }
}

/// Lit et vérifie un export d'audit, et le compare aux `attentes` (tête, cabinet, dossier).
///
/// # Errors
/// Défaut de forme (avec la ligne) ou erreur d'entrée-sortie.
pub fn verifier_export_audit<R: BufRead>(
    entree: R,
    attentes: &AttentesAudit,
) -> Result<RapportExportAudit, ErreurLecture> {
    let mut l = Lignes {
        entree,
        ligne: 0,
        tampon: Vec::with_capacity(LIGNE_MAX + 1),
    };
    if l.requise()? != ENTETE_EXPORT_AUDIT {
        return Err(l.erreur(ErreurExport::Attendu(ENTETE_EXPORT_AUDIT)));
    }
    let dossier = match l.valeur("portee")?.as_str() {
        "cabinet" => false,
        "dossier" => true,
        _ => {
            return Err(l.erreur(ErreurExport::Attendu(
                "portee=cabinet » ou « portee=dossier",
            )));
        }
    };
    let cabinet = l.uuid("cabinet")?;
    let portee = if dossier {
        Portee::Dossier {
            cabinet,
            dossier: l.uuid("dossier")?,
        }
    } else {
        Portee::Cabinet { cabinet }
    };
    let mut verificateur = Verificateur::nouveau(portee);
    let mut acteurs = BTreeSet::new();
    let (tete, nombre) = loop {
        let premiere = l.requise()?;
        if let Some(tete) = premiere.strip_prefix("tete=") {
            let tete =
                Empreinte::parse(tete).map_err(|_| l.erreur(ErreurExport::EmpreinteInvalide))?;
            let nombre = l.valeur("nombre")?;
            let nombre =
                lire_nombre(&nombre).ok_or_else(|| l.erreur(ErreurExport::NombreInvalide))?;
            break (tete, nombre);
        }
        if premiere != entree::ENTETE {
            return Err(l.erreur(ErreurExport::Attendu("audit v1 » ou « tete=")));
        }
        let mut contenu = premiere;
        contenu.push('\n');
        loop {
            let ligne = l.requise()?;
            if contenu.len() + ligne.len() + 1 > ENTREE_MAX {
                return Err(l.erreur(ErreurExport::EntreeAuditTropGrande));
            }
            contenu.push_str(&ligne);
            contenu.push('\n');
            if ligne == entree::FIN {
                break;
            }
        }
        let empreinte = l.valeur("empreinte")?;
        let empreinte =
            Empreinte::parse(&empreinte).map_err(|_| l.erreur(ErreurExport::EmpreinteInvalide))?;
        if let Some(lue) = verificateur.entree(&contenu, &empreinte, None)
            && let Acteur::Utilisateur { id, .. } = lue.evenement().acteur()
        {
            acteurs.insert(*id);
        }
    };
    let chaine = verificateur.terminer(Some(&tete), Some(nombre));

    let annexe = l.annexe()?;
    if l.requise()? != FIN_EXPORT {
        return Err(l.erreur(ErreurExport::Attendu(FIN_EXPORT)));
    }
    let mut reste = [0u8; 1];
    if l.entree.read(&mut reste)? != 0 {
        l.ligne += 1;
        return Err(l.erreur(ErreurExport::DonneesApresFin));
    }
    let annexe_coherente = annexe.iter().map(|(a, _)| *a).eq(acteurs.iter().copied());
    let ancrage_different = attentes.tete.is_some_and(|a| a != chaine.tete);
    let portee_inattendue = attentes.cabinet.is_some_and(|c| c != portee.cabinet())
        || attentes
            .dossier
            .is_some_and(|d| Some(d) != portee.dossier());
    Ok(RapportExportAudit {
        chaine,
        annexe,
        annexe_coherente,
        ancrage_different,
        portee_inattendue,
    })
}

/// Lecture ligne à ligne bornée (mêmes règles que l'export d'intégrité, `format-v1.md` §4).
struct Lignes<R: BufRead> {
    entree: R,
    ligne: u64,
    tampon: Vec<u8>,
}

impl<R: BufRead> Lignes<R> {
    fn erreur(&self, erreur: ErreurExport) -> ErreurLecture {
        ErreurLecture::Forme {
            ligne: self.ligne,
            erreur,
        }
    }

    fn requise(&mut self) -> Result<String, ErreurLecture> {
        self.tampon.clear();
        let lus = (&mut self.entree)
            .take(LIGNE_MAX as u64)
            .read_until(b'\n', &mut self.tampon)?;
        self.ligne += 1;
        if lus == 0 {
            return Err(self.erreur(ErreurExport::Tronque));
        }
        if self.tampon.last() != Some(&b'\n') {
            return Err(self.erreur(if lus == LIGNE_MAX {
                ErreurExport::LigneTropLongue
            } else {
                ErreurExport::FinDeLigneManquante
            }));
        }
        self.tampon.pop();
        String::from_utf8(std::mem::take(&mut self.tampon))
            .map_err(|_| self.erreur(ErreurExport::PasUtf8))
    }

    fn valeur(&mut self, cle: &'static str) -> Result<String, ErreurLecture> {
        let l = self.requise()?;
        match l.split_once('=') {
            Some((k, v)) if k == cle => Ok(v.to_owned()),
            _ => Err(self.erreur(ErreurExport::Attendu(cle))),
        }
    }

    /// Annexe des acteurs : UUID strictement croissants, nom facultatif non vide.
    fn annexe(&mut self) -> Result<Vec<(Uuid, Option<Texte>)>, ErreurLecture> {
        if self.requise()? != ANNEXE {
            return Err(self.erreur(ErreurExport::Attendu(ANNEXE)));
        }
        let mut annexe: Vec<(Uuid, Option<Texte>)> = Vec::new();
        loop {
            let ligne = self.requise()?;
            if ligne == FIN_ANNEXE {
                break;
            }
            if let Some(acteur) = ligne.strip_prefix("acteur=") {
                let acteur =
                    Uuid::parse(acteur).map_err(|_| self.erreur(ErreurExport::UuidInvalide))?;
                if annexe
                    .last()
                    .is_some_and(|(precedent, _)| *precedent >= acteur)
                {
                    return Err(self.erreur(ErreurExport::AnnexeInvalide));
                }
                annexe.push((acteur, None));
            } else if let Some(nom) = ligne.strip_prefix("nom=") {
                let nom = Texte::parse(nom)
                    .ok()
                    .filter(|n| !n.as_str().is_empty())
                    .ok_or_else(|| self.erreur(ErreurExport::AnnexeInvalide))?;
                match annexe.last_mut() {
                    Some((_, place @ None)) => *place = Some(nom),
                    _ => return Err(self.erreur(ErreurExport::AnnexeInvalide)),
                }
            } else {
                return Err(self.erreur(ErreurExport::Attendu("acteur », « nom » ou « fin-annexe")));
            }
        }
        Ok(annexe)
    }

    fn uuid(&mut self, cle: &'static str) -> Result<Uuid, ErreurLecture> {
        let v = self.valeur(cle)?;
        Uuid::parse(&v).map_err(|_| self.erreur(ErreurExport::UuidInvalide))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::chaine::{AnomalieAudit, ChaineAudit};
    use crate::audit::entree::{DecisionCedar, Evenement, Ressource, Symbole};

    const PORTEE: Portee = Portee::Dossier {
        cabinet: Uuid::depuis_octets([1; 16]),
        dossier: Uuid::depuis_octets([2; 16]),
    };

    fn acteur(n: u8) -> Acteur {
        Acteur::Utilisateur {
            id: Uuid::depuis_octets([n; 16]),
            role: Symbole::parse("valideur").unwrap(),
            session: Uuid::depuis_octets([0x50 + n; 16]),
            decision: DecisionCedar::new(
                vec![Symbole::parse("s20-exporter").unwrap()],
                Empreinte::calculer(b"v"),
                true,
            )
            .unwrap(),
        }
    }

    fn export(acteurs: &[Acteur], noms: &BTreeMap<Uuid, Texte>) -> String {
        let mut chaine = ChaineAudit::ouvrir(PORTEE);
        let mut e = EcrivainExportAudit::nouveau(PORTEE, Vec::new()).unwrap();
        for (i, a) in acteurs.iter().enumerate() {
            let entree = EntreeAudit::nouvelle(
                PORTEE,
                i as u64 + 1,
                time::macros::datetime!(2026-11-19 10:00 UTC),
                Evenement::new(
                    a.clone(),
                    Symbole::parse("ExporterComplet").unwrap(),
                    Ressource {
                        genre: Symbole::parse("dossier").unwrap(),
                        id: Uuid::depuis_octets([2; 16]),
                    },
                ),
            )
            .unwrap();
            let (contenu, empreinte) = chaine.ajouter(&entree).unwrap();
            e.entree(&contenu, &empreinte).unwrap();
        }
        let (sortie, _) = e.terminer(chaine.tete(), noms).unwrap();
        String::from_utf8(sortie).unwrap()
    }

    fn noms() -> BTreeMap<Uuid, Texte> {
        BTreeMap::from([(
            Uuid::depuis_octets([3; 16]),
            Texte::parse("Awa Ndiaye").unwrap(),
        )])
    }

    #[test]
    fn aller_retour_integre() {
        let texte = export(&[acteur(3), Acteur::Systeme, acteur(4)], &noms());
        let r = verifier_export_audit(texte.as_bytes(), &AttentesAudit::default()).unwrap();
        assert!(r.est_integre(), "{r}");
        assert_eq!(r.chaine.nombre, 3);
        assert_eq!(r.annexe.len(), 2);
        assert_eq!(r.annexe[0].1.as_ref().unwrap().as_str(), "Awa Ndiaye");
        assert_eq!(r.annexe[1].1, None);
        // L'annexe n'est pas chaînée : changer un nom ne change pas la chaîne.
        let renomme = texte.replacen("nom=Awa Ndiaye", "nom=Autre nom", 1);
        assert!(
            verifier_export_audit(renomme.as_bytes(), &AttentesAudit::default())
                .unwrap()
                .est_integre()
        );
        // Ancrage externe.
        let r2 = verifier_export_audit(
            texte.as_bytes(),
            &AttentesAudit {
                tete: Some(Empreinte::calculer(b"x")),
                ..AttentesAudit::default()
            },
        )
        .unwrap();
        assert!(r2.ancrage_different && !r2.est_integre());
    }

    #[test]
    fn alteration_et_annexe_incoherente_detectees() {
        let texte = export(&[acteur(3), acteur(4)], &noms());
        let altere = texte.replacen("reauthentification=oui", "reauthentification=non", 1);
        let r = verifier_export_audit(altere.as_bytes(), &AttentesAudit::default()).unwrap();
        assert!(
            r.chaine
                .anomalies
                .contains(&AnomalieAudit::EmpreinteAlteree { position: 1 })
        );
        let sans_acteur = texte.replacen("acteur=04040404-0404-0404-0404-040404040404\n", "", 1);
        let r = verifier_export_audit(sans_acteur.as_bytes(), &AttentesAudit::default()).unwrap();
        assert!(!r.annexe_coherente && !r.est_integre());
    }

    #[test]
    fn formes_refusees() {
        let texte = export(&[acteur(3)], &noms());
        for (avant, apres) in [
            (
                "sigma-books export-audit v1\n",
                "sigma-books export-audit v2\n",
            ),
            ("portee=dossier", "portee=Dossier"),
            ("fin-export\n", "fin-export\nx"),
            ("fin-export\n", "fin-export"),
            ("fin-annexe\n", ""),
            ("nombre=1", "nombre=01"),
            ("nom=Awa Ndiaye\n", "nom=Awa Ndiaye\nnom=Bis\n"),
            ("nom=Awa Ndiaye", "nom="),
        ] {
            let modifie = texte.replacen(avant, apres, 1);
            assert!(
                verifier_export_audit(modifie.as_bytes(), &AttentesAudit::default()).is_err(),
                "{avant} → {apres}"
            );
        }
        let longue = texte.replacen("nom=Awa Ndiaye", &format!("nom={}", "a".repeat(1100)), 1);
        assert!(matches!(
            verifier_export_audit(longue.as_bytes(), &AttentesAudit::default()),
            Err(ErreurLecture::Forme {
                erreur: ErreurExport::LigneTropLongue,
                ..
            })
        ));
    }

    #[test]
    fn l_ecrivain_refuse_une_entree_illisible_ou_d_une_autre_portee() {
        let mut e = EcrivainExportAudit::nouveau(PORTEE, Vec::new()).unwrap();
        let faux = "audit v1\nfin\nempreinte=00\naudit v1\n";
        assert!(e.entree(faux, &Empreinte::calculer(b"")).is_err());
        let autre = Portee::Cabinet {
            cabinet: Uuid::depuis_octets([1; 16]),
        };
        let entree = EntreeAudit::nouvelle(
            autre,
            1,
            time::macros::datetime!(2026-11-19 10:00 UTC),
            Evenement::new(
                Acteur::Systeme,
                Symbole::parse("A").unwrap(),
                Ressource {
                    genre: Symbole::parse("cabinet").unwrap(),
                    id: Uuid::depuis_octets([1; 16]),
                },
            ),
        )
        .unwrap();
        assert!(
            e.entree(&entree.serialiser(), &Empreinte::calculer(b""))
                .is_err()
        );
    }
}
