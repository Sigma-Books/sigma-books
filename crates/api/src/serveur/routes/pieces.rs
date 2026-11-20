//! Routes des pièces justificatives (PR 12 de `docs/lots/L2.md` ; ADR 0015) : dépôt d'une pièce
//! externe, création d'une pièce interne, liste paginée, téléchargement, écart d'un dépôt
//! erroné. Stockage, signature et forme canonique : [`crate::pieces`].
//!
//! Défense en profondeur (ADR 0011) : même si Cedar se trompait, chaque cas d'usage relit le
//! dossier **visible** (affectation, sauf administrateur) et, pour une mutation, exige un rôle
//! qui dépose des pièces (§20 : administrateur, valideur, collaborateur), lu en base. Toute
//! lecture de `piece` filtre par le cabinet et le dossier du contexte, sous RLS : la pièce d'un
//! autre dossier donne le même 404 qu'une pièce inexistante (F1.8).

use std::fmt::Write as _;
use std::time::Duration;

use authz::Action;
use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use integrite::Empreinte;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{lire_appartenance, lire_dossier_visible};
use crate::pieces::base::{
    NouvellePiece, Piece, ecarter_piece, inserer_piece, lire_piece, lister_pieces,
    pieces_de_meme_empreinte,
};
use crate::pieces::interne::{self, NoteOd};
use crate::pieces::{CleObjet, Nature, ObjetADeposer, Pieces, TypePiece};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi};
use crate::serveur::journal::CIBLE;
use crate::serveur::pipeline::{DossierAutorise, Garde, id_chemin};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Dépôt d'une pièce externe (corps : le fichier).
    Deposer,
    /// Création d'une pièce interne (note d'opérations diverses).
    CreerInterne,
    /// Pièces non écartées du dossier, paginées.
    Lister,
    /// Contenu d'une pièce, en pièce jointe, après recalcul de l'empreinte.
    Telecharger,
    /// Écart d'une pièce déposée par erreur, avec un motif.
    Ecarter,
}

/// Tranche du module. Champ de journal propre : `alerte` (liste fermée : `piece_alteree`,
/// `piece_absente`), émis quand l'objet relu ne correspond plus à la pièce (F3.1).
pub const MODULE: Module = Module {
    nom: "pieces",
    routes: ROUTES,
    champs_journal: &["alerte"],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // §20 « Déposer des pièces » : administrateur, valideur, collaborateur.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces",
        action: ActionRoute::Cabinet(Action::DeposerPiece),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Pieces(Cas::Deposer),
    },
    // Pièce interne (note d'OD) : mêmes rôles que le dépôt.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/internes",
        action: ActionRoute::Cabinet(Action::DeposerPiece),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Pieces(Cas::CreerInterne),
    },
    // Consultation : les rôles qui consultent les éditions (§20, quatre rôles du cabinet).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces",
        action: ActionRoute::Cabinet(Action::ConsulterEditions),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Pieces(Cas::Lister),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/{piece}/contenu",
        action: ActionRoute::Cabinet(Action::ConsulterEditions),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Pieces(Cas::Telecharger),
    },
    // Écart d'un dépôt erroné : les rôles qui déposent.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/pieces/{piece}/ecart",
        action: ActionRoute::Cabinet(Action::DeposerPiece),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Pieces(Cas::Ecarter),
    },
];

/// Limites propres d'une route du module (taille du corps, délai) : le dépôt reçoit une pièce
/// entière, bornée **pendant la réception du flux** par la taille maximale des pièces.
#[must_use]
pub(crate) fn limites(cas: Cas, pieces: Option<&Pieces>) -> Option<(usize, Duration)> {
    match (cas, pieces) {
        (Cas::Deposer, Some(p)) => Some((p.reglages().taille_max, p.reglages().delai_depot)),
        _ => None,
    }
}

/// Taille d'une page de liste par défaut et maximale.
const PAGE_PAR_DEFAUT: u32 = 50;
const PAGE_MAX: u32 = 100;

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let dossier = dossier_autorise(tx, garde).await?;
    match cas {
        Cas::Deposer => {
            exiger_deposant(tx).await?;
            deposer(tx, &dossier, requete).await
        }
        Cas::CreerInterne => {
            exiger_deposant(tx).await?;
            creer_interne(tx, &dossier, requete).await
        }
        Cas::Lister => lister(tx, &dossier, requete).await,
        Cas::Telecharger => telecharger(tx, &dossier, requete).await,
        Cas::Ecarter => {
            exiger_deposant(tx).await?;
            ecarter(tx, &dossier, requete).await
        }
    }
}

/// Le dossier du chemin, relu **visible** (affectation, sauf administrateur) : même si Cedar
/// se trompait, un dossier non affecté reste absent (404).
async fn dossier_autorise(
    tx: &mut TransactionTenant,
    garde: &Garde,
) -> Result<DossierAutorise, ErreurApi> {
    let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
    let dossier = lire_dossier_visible(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    garde.dossier_du_chemin(dossier)
}

/// Défense en profondeur des mutations : appartenance **active**, lue en base, d'un rôle qui
/// dépose des pièces (§20). Sinon 403 (le dossier est visible).
async fn exiger_deposant(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    let autorises = [
        authz::Role::Administrateur,
        authz::Role::Valideur,
        authz::Role::Collaborateur,
    ];
    if autorises.iter().any(|r| r.nom() == appartenance.role) {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

fn service<'a>(requete: &Requete<'a>) -> Result<&'a Pieces, ErreurApi> {
    requete
        .pieces
        .ok_or(ErreurApi::Interne(CategorieErreur::Stockage))
}

/// Instant de l'horloge, à la microseconde (précision de PostgreSQL et de la forme canonique).
fn maintenant() -> OffsetDateTime {
    let i = OffsetDateTime::now_utc();
    i.replace_nanosecond(i.nanosecond() / 1000 * 1000)
        .unwrap_or(i)
}

// --- Paramètres de l'URL ------------------------------------------------------------------

/// Paramètres de l'URL décodés (`%xx`, `+` pour l'espace), en UTF-8 strict ; `admis` liste les
/// seuls noms acceptés, chacun au plus une fois. Sinon 400, sans écho.
fn parametres(
    texte: Option<&str>,
    admis: &[&'static str],
) -> Result<Vec<(&'static str, String)>, ErreurApi> {
    let mut lus: Vec<(&'static str, String)> = Vec::new();
    for paire in texte.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (cle, valeur) = paire.split_once('=').ok_or(ErreurApi::RequeteInvalide)?;
        let nom = admis
            .iter()
            .find(|a| **a == cle)
            .ok_or(ErreurApi::RequeteInvalide)?;
        if lus.iter().any(|(n, _)| n == nom) {
            return Err(ErreurApi::RequeteInvalide);
        }
        lus.push((nom, decoder(valeur).ok_or(ErreurApi::RequeteInvalide)?));
    }
    Ok(lus)
}

fn decoder(texte: &str) -> Option<String> {
    let mut octets = Vec::with_capacity(texte.len());
    let mut i = texte.bytes();
    while let Some(o) = i.next() {
        match o {
            b'+' => octets.push(b' '),
            b'%' => {
                let fort = char::from(i.next()?).to_digit(16)?;
                let faible = char::from(i.next()?).to_digit(16)?;
                octets.push(u8::try_from(fort * 16 + faible).ok()?);
            }
            o => octets.push(o),
        }
    }
    String::from_utf8(octets).ok()
}

fn valeur<'a>(lus: &'a [(&'static str, String)], nom: &str) -> Option<&'a str> {
    lus.iter()
        .find_map(|(n, v)| (*n == nom).then_some(v.as_str()))
}

/// Nom de fichier d'origine admis : 1 à 255 octets UTF-8, sans caractère de contrôle. Il reste
/// une métadonnée ; le chemin éventuel n'est pas interprété.
fn nom_origine(texte: &str) -> Result<String, ErreurApi> {
    if (1..=255).contains(&texte.len()) && !texte.chars().any(char::is_control) {
        Ok(texte.to_owned())
    } else {
        Err(ErreurApi::RequeteInvalide)
    }
}

/// Texte d'un motif : non blanc, 1 à 1 000 octets, sans caractère de contrôle.
fn motif(texte: &str) -> Result<String, ErreurApi> {
    if !texte.trim().is_empty() && texte.len() <= 1000 && !texte.chars().any(char::is_control) {
        Ok(texte.to_owned())
    } else {
        Err(ErreurApi::RequeteInvalide)
    }
}

// --- Vues -----------------------------------------------------------------------------------

/// Vue d'une pièce : ne se construit que pour le dossier décidé ([`DossierAutorise`]), depuis
/// une ligne relue sous son contexte.
#[derive(Debug, Serialize)]
struct VuePiece {
    id: String,
    nature: &'static str,
    #[serde(rename = "type")]
    type_piece: &'static str,
    taille: u64,
    empreinte: String,
    nom: Option<String>,
    deposee_par: String,
    deposee_le: String,
    retenue_jusqu_au: String,
    ecartee: bool,
}

impl VuePiece {
    fn new(_dossier: &DossierAutorise, piece: &Piece) -> Self {
        Self {
            id: piece.id.to_string(),
            nature: piece.nature.nom(),
            type_piece: piece.type_piece.mime(),
            taille: piece.taille,
            empreinte: piece.empreinte.to_string(),
            nom: piece.nom_origine.clone(),
            deposee_par: piece.deposee_par.to_string(),
            deposee_le: interne::instant(piece.deposee_le),
            retenue_jusqu_au: interne::instant(piece.retenue_jusqu_au),
            ecartee: piece.ecart.is_some(),
        }
    }
}

/// Avertissement de doublon : les pièces du **même dossier** de même empreinte.
#[derive(Debug, Serialize)]
struct VueDoublon {
    doublon: bool,
    pieces: Vec<VuePiece>,
}

/// Page de la liste.
#[derive(Debug, Serialize)]
struct VuePage {
    pieces: Vec<VuePiece>,
    /// Curseur de la page suivante (`apres=`), absent à la dernière page.
    suivant: Option<String>,
}

// --- Dépôt -----------------------------------------------------------------------------------

/// Pièce prête à être stockée : contenu, type et empreinte établis par le serveur.
struct Contenu<'a> {
    octets: &'a [u8],
    type_piece: TypePiece,
    empreinte: Empreinte,
    nature: Nature,
    nom_origine: Option<String>,
}

/// Dépôt d'une pièce externe. Paramètres : `nom` (nom d'origine), `empreinte` (SHA-256
/// hexadécimale annoncée par le client, refus si elle diffère), `doublon=confirme`.
async fn deposer(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let pieces = service(requete)?;
    let lus = parametres(requete.parametres, &["nom", "empreinte", "doublon"])?;
    let nom = valeur(&lus, "nom").map(nom_origine).transpose()?;
    let confirme = match valeur(&lus, "doublon") {
        None => false,
        Some("confirme") => true,
        Some(_) => return Err(ErreurApi::RequeteInvalide),
    };
    let octets = requete.corps;
    if octets.is_empty() {
        return Err(ErreurApi::RequeteInvalide);
    }
    if octets.len() > pieces.reglages().taille_max {
        return Err(ErreurApi::CorpsTropGrand);
    }
    // Signature seule : ni l'extension du nom, ni le type annoncé par le client.
    let type_piece = TypePiece::reconnaitre(octets).ok_or(ErreurApi::TypeNonAdmis)?;
    let empreinte = Empreinte::calculer(octets);
    if let Some(annoncee) = valeur(&lus, "empreinte") {
        let annoncee = Empreinte::parse(&annoncee.to_ascii_lowercase())
            .map_err(|_| ErreurApi::RequeteInvalide)?;
        if annoncee != empreinte {
            return Err(ErreurApi::RequeteInvalide);
        }
    }
    // Doublons : dans le dossier du contexte seulement (spec §9.3, F3.5).
    let doublons = pieces_de_meme_empreinte(tx, &empreinte).await?;
    if !doublons.is_empty() && !confirme {
        let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "piece", doublons[0].id);
        trace.donnees.push(("issue", "doublon_signale".to_owned()));
        trace.donnees.push(("empreinte", empreinte.to_string()));
        let vue = VueDoublon {
            doublon: true,
            pieces: doublons.iter().map(|p| VuePiece::new(dossier, p)).collect(),
        };
        return Ok(Resultat {
            reponse: (StatusCode::OK, Json(vue)).into_response(),
            traces: vec![trace],
        });
    }
    let contenu = Contenu {
        octets,
        type_piece,
        empreinte,
        nature: Nature::Externe,
        nom_origine: nom,
    };
    let confirmes: Vec<String> = doublons.iter().map(|p| p.id.to_string()).collect();
    stocker(tx, dossier, pieces, contenu, None, &confirmes).await
}

/// Écrit l'objet, **puis** la ligne (ADR 0015), et rend la trace de l'action.
async fn stocker(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    pieces: &Pieces,
    contenu: Contenu<'_>,
    motif_trace: Option<String>,
    doublon_de: &[String],
) -> Result<Resultat, ErreurApi> {
    let cabinet = tx.contexte().id_cabinet();
    let instant = maintenant();
    let retenir = pieces.fin_de_retention(instant);
    let cle = CleObjet::nouvelle(cabinet, dossier.id());
    let depot = pieces
        .stockage()
        .deposer(ObjetADeposer {
            cle: &cle,
            contenu: contenu.octets,
            empreinte: contenu.empreinte,
            type_contenu: contenu.type_piece.mime(),
            retenir_jusqu_au: retenir,
        })
        .await
        .map_err(|_| ErreurApi::Interne(CategorieErreur::Stockage))?;
    let taille = u64::try_from(contenu.octets.len()).map_err(|_| ErreurApi::CorpsTropGrand)?;
    let nouvelle = NouvellePiece {
        id: Uuid::new_v4(),
        nature: contenu.nature,
        type_piece: contenu.type_piece,
        taille,
        empreinte: contenu.empreinte,
        nom_origine: contenu.nom_origine,
        cle,
        version: depot.version,
        retenue_jusqu_au: retenir,
        deposee_le: instant,
    };
    inserer_piece(tx, &nouvelle).await?;
    let piece = lire_piece(tx, nouvelle.id)
        .await?
        .ok_or(ErreurApi::Interne(CategorieErreur::Donnee))?;

    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "piece", piece.id);
    trace.motif = motif_trace;
    let mut champ =
        |nom: &'static str, valeur: String| trace.champs.push((nom, None, Some(valeur)));
    champ("nature", piece.nature.nom().to_owned());
    champ("type", piece.type_piece.mime().to_owned());
    champ("taille", piece.taille.to_string());
    champ("empreinte", piece.empreinte.to_string());
    if let Some(nom) = &piece.nom_origine {
        champ("nom", nom.clone());
    }
    champ("retenue_jusqu_au", interne::instant(piece.retenue_jusqu_au));
    if !doublon_de.is_empty() {
        // Ajout confirmé malgré l'avertissement : choix journalisé (spec §9.3).
        trace
            .donnees
            .push(("doublon_confirme_de", doublon_de.join(",")));
    }
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(VuePiece::new(dossier, &piece))).into_response(),
        traces: vec![trace],
    })
}

/// Corps de la création d'une pièce interne.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NouvelleNote {
    /// Date de l'opération, `AAAA-MM-JJ`.
    date: String,
    motif: String,
    #[serde(default)]
    lignes: Vec<String>,
}

/// Pièce interne : note d'OD sous forme canonique v1, hachée et stockée comme une pièce
/// externe. Aucune recherche de doublon (l'auteur et l'instant la rendent unique).
async fn creer_interne(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let pieces = service(requete)?;
    parametres(requete.parametres, &[])?;
    let Json(NouvelleNote {
        date,
        motif,
        lignes,
    }) = Json::<NouvelleNote>::from_bytes(requete.corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    let date = interne::lire_date(&date).map_err(|_| ErreurApi::RequeteInvalide)?;
    let contexte = *tx.contexte();
    let note = NoteOd::new(
        contexte.id_cabinet().uuid(),
        dossier.id().uuid(),
        contexte.id_utilisateur().uuid(),
        maintenant(),
        date,
        motif,
        lignes,
    )
    .map_err(|_| ErreurApi::RequeteInvalide)?;
    let texte = note.serialiser();
    let contenu = Contenu {
        octets: texte.as_bytes(),
        type_piece: TypePiece::NoteOd,
        empreinte: Empreinte::calculer(texte.as_bytes()),
        nature: Nature::Interne,
        nom_origine: Some(format!("note-od-{}.txt", interne::date(note.date()))),
    };
    let motif = note.motif().to_owned();
    stocker(tx, dossier, pieces, contenu, Some(motif), &[]).await
}

// --- Consultation -----------------------------------------------------------------------------

/// Liste paginée : `apres=<pièce>` (curseur), `limite=` (1 à 100, 50 par défaut).
async fn lister(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let lus = parametres(requete.parametres, &["apres", "limite"])?;
    let apres = valeur(&lus, "apres")
        .map(|v| Uuid::try_parse(v).map_err(|_| ErreurApi::RequeteInvalide))
        .transpose()?;
    let limite = match valeur(&lus, "limite") {
        None => PAGE_PAR_DEFAUT,
        Some(v) if !v.is_empty() && v.bytes().all(|o| o.is_ascii_digit()) => v
            .parse::<u32>()
            .ok()
            .filter(|l| (1..=PAGE_MAX).contains(l))
            .ok_or(ErreurApi::RequeteInvalide)?,
        Some(_) => return Err(ErreurApi::RequeteInvalide),
    };
    let page = lister_pieces(tx, apres, limite).await?;
    let suivant = (page.len() == limite as usize)
        .then(|| page.last().map(|p| p.id.to_string()))
        .flatten();
    let vue = VuePage {
        pieces: page.iter().map(|p| VuePiece::new(dossier, p)).collect(),
        suivant,
    };
    Ok(Resultat {
        reponse: Json(vue).into_response(),
        traces: Vec::new(),
    })
}

/// Pièce du chemin (`{piece}`) : 400 si ce n'est pas un UUID, 404 si elle n'est pas dans le
/// dossier du contexte.
async fn piece_du_chemin(
    tx: &mut TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Piece, ErreurApi> {
    let id = id_chemin(requete.chemin, "piece")?;
    lire_piece(tx, id).await?.ok_or(ErreurApi::Introuvable)
}

/// Téléchargement : la version écrite de l'objet, relue en entier, son empreinte **recalculée**.
/// Écart (objet altéré, remplacé ou absent) : refus, alerte au journal technique et entrée au
/// journal d'audit, validée malgré le refus (F3.1).
async fn telecharger(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    let pieces = service(requete)?;
    parametres(requete.parametres, &[])?;
    let piece = piece_du_chemin(tx, requete).await?;
    let taille =
        usize::try_from(piece.taille).map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    let lu = pieces
        .stockage()
        .lire(&piece.cle, &piece.version, taille)
        .await;
    let (alerte, empreinte_lue) = match lu {
        Ok(octets) => {
            let lue = Empreinte::calculer(&octets);
            if lue == piece.empreinte && octets.len() == taille {
                return Ok(Resultat {
                    reponse: piece_jointe(&piece, octets),
                    traces: Vec::new(),
                });
            }
            ("piece_alteree", Some(lue))
        }
        Err(crate::pieces::ErreurStockage::Absent) => ("piece_absente", None),
        Err(crate::pieces::ErreurStockage::TropGrand) => ("piece_alteree", None),
        Err(_) => return Err(ErreurApi::Interne(CategorieErreur::Stockage)),
    };
    tracing::error!(
        target: CIBLE,
        alerte,
        "alerte de sécurité : pièce justificative non conforme à son empreinte"
    );
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "piece", piece.id);
    trace.donnees.push(("alerte", alerte.to_owned()));
    trace
        .donnees
        .push(("empreinte_attendue", piece.empreinte.to_string()));
    if let Some(lue) = empreinte_lue {
        trace.donnees.push(("empreinte_lue", lue.to_string()));
    }
    Ok(Resultat {
        reponse: ErreurApi::Interne(CategorieErreur::Stockage).into_response(),
        traces: vec![trace],
    })
}

/// Réponse de téléchargement : toujours en pièce jointe (RFC 6266), type de la signature,
/// `nosniff` ; la CSP `sandbox; default-src 'none'` de l'API est posée par
/// [`crate::serveur::entetes`].
fn piece_jointe(piece: &Piece, octets: Vec<u8>) -> Response {
    let nom = piece
        .nom_origine
        .clone()
        .unwrap_or_else(|| format!("piece-{}.{}", piece.id, piece.type_piece.extension()));
    let mut reponse = (StatusCode::OK, octets).into_response();
    let entetes = reponse.headers_mut();
    entetes.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(piece.type_piece.mime()),
    );
    if let Ok(valeur) = HeaderValue::from_str(&disposition(&nom)) {
        entetes.insert(header::CONTENT_DISPOSITION, valeur);
    } else {
        entetes.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }
    entetes.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    reponse
}

/// `Content-Disposition: attachment` (RFC 6266) : `filename` de repli en ASCII sûr, puis
/// `filename*` en UTF-8 encodé selon RFC 8187.
fn disposition(nom: &str) -> String {
    let repli: String = nom
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_' | '(' | ')' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encode = String::new();
    for o in nom.bytes() {
        if o.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&o) {
            encode.push(char::from(o));
        } else {
            let _ = write!(encode, "%{o:02X}");
        }
    }
    format!("attachment; filename=\"{repli}\"; filename*=UTF-8''{encode}")
}

// --- Écart ------------------------------------------------------------------------------------

/// Corps de l'écart.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsEcart {
    motif: String,
}

/// Écart d'une pièce déposée par erreur : elle disparaît des listes, reste téléchargeable et
/// stockée sous verrou jusqu'au terme de sa rétention. Une pièce déjà écartée est absente.
async fn ecarter(
    tx: &mut TransactionTenant,
    dossier: &DossierAutorise,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    parametres(requete.parametres, &[])?;
    let Json(CorpsEcart { motif: texte }) =
        Json::<CorpsEcart>::from_bytes(requete.corps).map_err(|_| ErreurApi::RequeteInvalide)?;
    let motif = motif(&texte)?;
    let id = id_chemin(requete.chemin, "piece")?;
    let piece = ecarter_piece(tx, id, &motif, maintenant())
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier.id()), "piece", piece.id);
    trace.motif = Some(motif);
    trace
        .champs
        .push(("ecartee", Some("non".to_owned()), Some("oui".to_owned())));
    Ok(Resultat {
        reponse: Json(VuePiece::new(dossier, &piece)).into_response(),
        traces: vec![trace],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parametres_stricts() {
        let lus = parametres(
            Some("nom=Facture%20n%C2%B01+mars.pdf&doublon=confirme"),
            &["nom", "doublon"],
        )
        .unwrap();
        assert_eq!(valeur(&lus, "nom"), Some("Facture n°1 mars.pdf"));
        assert_eq!(valeur(&lus, "doublon"), Some("confirme"));
        for refuse in [
            "nom=a&nom=b",
            "autre=1",
            "nom",
            "nom=%FF",
            "nom=%2",
            "nom=%G0",
        ] {
            assert_eq!(
                parametres(Some(refuse), &["nom", "doublon"]),
                Err(ErreurApi::RequeteInvalide),
                "{refuse}"
            );
        }
        assert_eq!(parametres(None, &[]), Ok(vec![]));
    }

    #[test]
    fn noms_et_motifs() {
        assert!(nom_origine("relevé.pdf").is_ok());
        assert!(nom_origine("").is_err());
        assert!(nom_origine("a\nb.pdf").is_err());
        assert!(nom_origine(&"a".repeat(256)).is_err());
        assert!(motif("Dépôt en double").is_ok());
        assert!(motif("   ").is_err());
        assert!(motif("a\u{7}").is_err());
    }

    #[test]
    fn disposition_rfc_6266() {
        assert_eq!(
            disposition("Facture n°1 \"mars\".pdf"),
            "attachment; filename=\"Facture n_1 _mars_.pdf\"; \
             filename*=UTF-8''Facture%20n%C2%B01%20%22mars%22.pdf"
        );
        assert_eq!(
            disposition("../../etc/passwd;x"),
            "attachment; filename=\".._.._etc_passwd_x\"; filename*=UTF-8''..%2F..%2Fetc%2Fpasswd%3Bx"
        );
    }

    #[test]
    fn seul_le_depot_a_des_limites_propres() {
        assert_eq!(limites(Cas::Deposer, None), None);
        for cas in [
            Cas::CreerInterne,
            Cas::Lister,
            Cas::Telecharger,
            Cas::Ecarter,
        ] {
            assert_eq!(limites(cas, None), None);
        }
    }
}
