//! Cas d'usage de l'intégrité d'un dossier (L2-31, PR 15b ; spec §6.4, §21 ; ADR 0007, 0010 ;
//! menaces F2.4, F2.5, F7.4) : état de la chaîne et des jetons, liste des jetons, vérification
//! depuis la base (#188), export d'intégrité (format en vigueur, ré-authentification, alerte,
//! audit), témoin externe.
//!
//! Toutes les lectures se font dans une transaction à instantané unique
//! ([`ouvrir_transaction_lecture`]) : tête, écritures et jetons sont d'un même état, même si une
//! validation a lieu pendant la lecture. Les écritures sont lues par pages
//! ([`crate::base::integrite::page_chaine`]) ; l'export est construit en mémoire (limite acceptée
//! en L2, comme l'export d'audit, `crates/api/README.md`).

use std::fmt::Write as _;
use std::sync::OnceLock;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64ct::{Base64, Encoding};
use integrite::export::{BASE64_LIGNE, ENTETE_EXPORT_V3, ENTETE_EXPORT_V4, FIN_EXPORT};
use integrite::horodatage::{Ancre, Temoin, ancres_par_defaut};
use integrite::{Empreinte, IdDossier, genese};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::audit::Trace;
use super::erreur::{CategorieErreur, ErreurApi};
use super::pipeline::Garde;
use super::routes::Requete;
use crate::base::audit::PorteeAudit;
use crate::base::integrite::{
    a_une_etiquette, compter_jetons, horodatage_en_attente_depuis, jetons, page_chaine,
};
use crate::base::jetons::lire_tete;
use crate::base::taches::{SujetNotification, notifier};
use crate::base::{TransactionTenant, ouvrir_transaction_lecture};
use crate::verification::{
    Anomalie, ControleOfficiel, JetonStocke, LigneChaine, Rapport, VerificateurChaine,
};

/// Écritures lues par page pendant la vérification et l'export.
const PAGE: i64 = 500;
/// Retard d'horodatage signalé (F6.2).
const RETARD: time::Duration = time::Duration::hours(24);

fn interne<T>(_: T) -> ErreurApi {
    ErreurApi::Interne(CategorieErreur::Donnee)
}

/// Dossier du contexte, au format de `integrite`.
fn id_dossier(tx: &TransactionTenant) -> Result<IdDossier, ErreurApi> {
    let dossier = tx.contexte().id_dossier().ok_or(interne(()))?;
    IdDossier::parse(&dossier.to_string()).map_err(interne)
}

fn instant(i: OffsetDateTime) -> Result<String, ErreurApi> {
    i.format(&Rfc3339).map_err(interne)
}

/// Ancres de confiance intégrées, lues une fois.
fn ancres() -> Result<&'static [Ancre], ErreurApi> {
    static ANCRES: OnceLock<Result<Vec<Ancre>, ()>> = OnceLock::new();
    ANCRES
        .get_or_init(|| ancres_par_defaut().map_err(|_| ()))
        .as_ref()
        .map(Vec::as_slice)
        .map_err(|()| interne(()))
}

/// Tête et nombre d'écritures du dossier : la chaîne vide a la tête de genèse.
async fn tete(tx: &mut TransactionTenant) -> Result<(i64, Empreinte), ErreurApi> {
    Ok(match lire_tete(tx).await? {
        Some(t) => (t.rang, t.empreinte),
        None => (0, genese(&id_dossier(tx)?)),
    })
}

/// Transaction de lecture à instantané unique, au contexte de la transaction de l'action.
async fn lecture(
    tx: &TransactionTenant,
    requete: &Requete<'_>,
) -> Result<TransactionTenant, ErreurApi> {
    Ok(ouvrir_transaction_lecture(requete.pool, *tx.contexte()).await?)
}

#[derive(Serialize)]
struct VueJeton {
    id: String,
    nombre: i64,
    tete: String,
    autorite: String,
    date_jeton: String,
    obtenu_le: String,
    declencheur: String,
}

fn vue_jeton(j: &JetonStocke) -> Result<VueJeton, ErreurApi> {
    Ok(VueJeton {
        id: j.id.to_string(),
        nombre: j.nombre,
        tete: j.tete.to_string(),
        autorite: j.autorite.clone(),
        date_jeton: instant(j.date_jeton)?,
        obtenu_le: instant(j.obtenu_le)?,
        declencheur: j.declencheur.clone(),
    })
}

#[derive(Serialize)]
struct VueTemoin {
    contenu: String,
    nombre: i64,
    tete: String,
}

#[derive(Serialize)]
struct VueHorodatage {
    /// Instant depuis lequel une écriture attend un jeton.
    en_attente_depuis: Option<String>,
    /// Vrai si l'attente dépasse 24 h (F6.2).
    retard: bool,
}

#[derive(Serialize)]
struct VueEtat {
    dossier: String,
    nombre: i64,
    tete: String,
    jetons: i64,
    dernier_jeton: Option<VueJeton>,
    ecritures_sans_jeton: i64,
    horodatage: VueHorodatage,
    temoin: VueTemoin,
}

fn temoin_de(tx: &TransactionTenant, nombre: i64, tete: Empreinte) -> Result<Temoin, ErreurApi> {
    Ok(Temoin::nouveau(
        id_dossier(tx)?,
        tete,
        u64::try_from(nombre).map_err(interne)?,
    ))
}

/// État de la chaîne : nombre d'écritures, tête, dernier jeton, écritures sans jeton, retard
/// d'horodatage et témoin courant. Ne vérifie aucune signature : voir [`verifier`].
///
/// # Errors
/// Erreur de base.
pub async fn etat(tx: &TransactionTenant, requete: &Requete<'_>) -> Result<Response, ErreurApi> {
    let mut l = lecture(tx, requete).await?;
    let (nombre, tete) = tete(&mut l).await?;
    let tous = jetons(&mut l).await?;
    let total = compter_jetons(&mut l).await?;
    let attente = horodatage_en_attente_depuis(&mut l).await?;
    let temoin = temoin_de(&l, nombre, tete)?;
    l.annuler().await?;
    let dernier = tous.iter().max_by_key(|j| (j.nombre, j.obtenu_le));
    let couvertes = dernier.map_or(0, |j| j.nombre);
    let retard = attente.is_some_and(|depuis| requete.maintenant - depuis > RETARD);
    let vue = VueEtat {
        dossier: id_dossier(tx)?.to_string(),
        nombre,
        tete: tete.to_string(),
        jetons: total,
        dernier_jeton: dernier.map(vue_jeton).transpose()?,
        ecritures_sans_jeton: (nombre - couvertes).max(0),
        horodatage: VueHorodatage {
            en_attente_depuis: attente.map(instant).transpose()?,
            retard,
        },
        temoin: VueTemoin {
            contenu: temoin.contenu(),
            nombre,
            tete: tete.to_string(),
        },
    };
    Ok((StatusCode::OK, Json(vue)).into_response())
}

#[derive(Serialize)]
struct VueListeJetons {
    total: i64,
    jetons: Vec<VueJeton>,
}

/// Jetons du dossier, du plus ancien au plus récent (au plus 10 000, les plus récents).
///
/// # Errors
/// Erreur de base.
pub async fn lister_jetons(
    tx: &TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Response, ErreurApi> {
    let mut l = lecture(tx, requete).await?;
    let tous = jetons(&mut l).await?;
    let total = compter_jetons(&mut l).await?;
    l.annuler().await?;
    let jetons = tous.iter().map(vue_jeton).collect::<Result<_, _>>()?;
    Ok((StatusCode::OK, Json(VueListeJetons { total, jetons })).into_response())
}

#[derive(Serialize)]
struct VueAnomalie {
    code: &'static str,
    rang: Option<i64>,
    numero: Option<String>,
    detail: Option<&'static str>,
}

impl From<&Anomalie> for VueAnomalie {
    fn from(a: &Anomalie) -> Self {
        Self {
            code: a.code,
            rang: a.rang,
            numero: a.numero.clone(),
            detail: a.detail,
        }
    }
}

#[derive(Serialize)]
struct VueVerification {
    integre: bool,
    nombre: u64,
    tete: String,
    jetons: u64,
    jetons_valides: u64,
    /// Rang de la première écriture altérée, s'il y en a une.
    premiere_alteration: Option<i64>,
    anomalies: Vec<VueAnomalie>,
    anomalies_omises: u64,
}

/// Recalcule la chaîne du dossier depuis la base et la compare à la tête stockée et aux jetons.
///
/// # Errors
/// Erreur de base.
pub async fn calculer(tx: &TransactionTenant, requete: &Requete<'_>) -> Result<Rapport, ErreurApi> {
    let mut l = lecture(tx, requete).await?;
    let dossier = id_dossier(&l)?;
    let stockee = lire_tete(&mut l).await?.map(|t| (t.rang, t.empreinte));
    let tous = jetons(&mut l).await?;
    let mut verificateur = VerificateurChaine::new(&dossier, tous.iter().map(|j| j.nombre));
    let mut apres = 0;
    loop {
        let page = page_chaine(&mut l, apres, PAGE).await?;
        let Some(derniere) = page.last() else { break };
        apres = derniere.rang;
        for ligne in &page {
            verificateur.ajouter(ligne);
        }
    }
    l.annuler().await?;
    // Les signatures RSA des jetons sont du calcul : hors du fil des requêtes.
    let ancres = ancres()?;
    tokio::task::spawn_blocking(move || {
        let controle = ControleOfficiel::new(ancres.to_vec());
        verificateur.terminer(&dossier, stockee, &tous, &controle)
    })
    .await
    .map_err(interne)
}

/// Vérification depuis la base (#188) : rapport « intègre » ou localisation des anomalies.
///
/// # Errors
/// Erreur de base.
pub async fn verifier(
    tx: &TransactionTenant,
    requete: &Requete<'_>,
) -> Result<Response, ErreurApi> {
    let r = calculer(tx, requete).await?;
    let vue = VueVerification {
        integre: r.est_integre(),
        nombre: r.nombre,
        tete: r.tete.to_string(),
        jetons: r.jetons,
        jetons_valides: r.jetons_valides,
        premiere_alteration: r
            .anomalies
            .iter()
            .filter(|a| a.code != "tete_differente" && a.code != "tete_rang_different")
            .find_map(|a| a.numero.as_ref().and(a.rang)),
        anomalies: r.anomalies.iter().map(VueAnomalie::from).collect(),
        anomalies_omises: r.anomalies_omises,
    };
    Ok((StatusCode::OK, Json(vue)).into_response())
}

fn piece_jointe(texte: String, nom: &str) -> Result<Response, ErreurApi> {
    let mut reponse = texte.into_response();
    let entetes = reponse.headers_mut();
    entetes.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    entetes.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{nom}\"")).map_err(interne)?,
    );
    Ok(reponse)
}

/// Témoin externe courant (`docs/integrite/format-v2.md` §4), en pièce jointe : dossier, tête et
/// nombre d'écritures. Aucune donnée comptable : à conserver hors de l'instance (F2.4).
///
/// # Errors
/// Erreur de base.
pub async fn temoin(tx: &TransactionTenant, requete: &Requete<'_>) -> Result<Response, ErreurApi> {
    let mut l = lecture(tx, requete).await?;
    let (nombre, tete) = tete(&mut l).await?;
    let temoin = temoin_de(&l, nombre, tete)?;
    l.annuler().await?;
    let nom = format!("temoin-{}.txt", id_dossier(tx)?);
    piece_jointe(temoin.contenu(), &nom)
}

/// Export d'intégrité du dossier : format en vigueur (v3, ou v4 si une écriture porte une
/// étiquette fiscale), tel que la base le contient (texte canonique et empreintes stockés, sans
/// rien recalculer : une altération de la base se voit alors à la vérification de l'export),
/// avec les jetons. Inscrit à l'audit (tête, nombre, jetons) ; crée l'alerte à l'administrateur
/// dans la même transaction.
///
/// # Errors
/// Erreur de base.
pub async fn exporter(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<(Response, Trace), ErreurApi> {
    let mut l = lecture(tx, requete).await?;
    let dossier = id_dossier(&l)?;
    let v4 = a_une_etiquette(&mut l).await?;
    let tous = jetons(&mut l).await?;
    let mut sortie = String::new();
    let _ = writeln!(
        sortie,
        "{}\ndossier={dossier}",
        if v4 {
            ENTETE_EXPORT_V4
        } else {
            ENTETE_EXPORT_V3
        }
    );
    let (mut nombre, mut tete) = (0_u64, genese(&dossier));
    let mut apres = 0;
    loop {
        let page: Vec<LigneChaine> = page_chaine(&mut l, apres, PAGE).await?;
        let Some(derniere) = page.last() else { break };
        apres = derniere.rang;
        for e in &page {
            sortie.push_str(&e.contenu);
            let _ = writeln!(sortie, "empreinte={}", e.empreinte);
            tete = e.empreinte;
            nombre += 1;
        }
    }
    l.annuler().await?;
    let _ = writeln!(sortie, "tete={tete}\nnombre={nombre}");
    for j in &tous {
        let _ = writeln!(sortie, "jeton\nnombre={}\ntete={}", j.nombre, j.tete);
        let base64 = Base64::encode_string(&j.der);
        for morceau in base64.as_bytes().chunks(BASE64_LIGNE) {
            sortie.push_str("der=");
            sortie.push_str(&String::from_utf8_lossy(morceau));
            sortie.push('\n');
        }
        sortie.push_str("fin-jeton\n");
    }
    let _ = writeln!(sortie, "{FIN_EXPORT}");

    // Alerte à l'administrateur (spec §21, F7.4) : dans la transaction de l'export.
    notifier(
        tx,
        SujetNotification::ExportIntegrite,
        Some(garde.utilisateur().uuid()),
    )
    .await?;
    let id = tx.contexte().id_dossier().ok_or(interne(()))?;
    let mut trace = Trace::new(PorteeAudit::Dossier(id), "dossier", id.uuid());
    trace.donnees.push(("export", "integrite".to_owned()));
    trace.donnees.push(("tete", tete.to_string()));
    trace.donnees.push(("nombre", nombre.to_string()));
    trace.donnees.push(("jetons", tous.len().to_string()));
    Ok((
        piece_jointe(sortie, &format!("integrite-dossier-{id}.txt"))?,
        trace,
    ))
}
