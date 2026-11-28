//! Aides de test du module de routes `releves` (lot L3, PR 12a) : corps d'exemple des mutations,
//! profil et dépôt fixes d'un dossier de test (sous-ressources `{profil}` et `{depot}`).
//!
//! Fichiers **synthétiques** (banque, libellés et montants inventés ; aucun identifiant bancaire).

#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

use api::base::{IdCabinet, IdDossier, IdUtilisateur};
use api::pieces::CleObjet;
use formats::releve::{ProfilBrut, ProfilReleve};
use integrite::Empreinte;
use sqlx::PgPool;
use uuid::Uuid;

use super::pieces::StockageMemoire;

/// Profil des fichiers de test : UTF-8, point-virgule, une ligne de titres, date `JJ/MM/AAAA`,
/// virgule décimale, montant signé et solde.
pub const PROFIL: &str = r#"{"encodage":"utf-8","delimiteur":";","lignes_entete":1,
"format_date":"%d/%m/%Y","separateur_decimal":",",
"colonnes":{"date":"Date","libelle":"Libellé","montant":"Montant","solde":"Solde"}}"#;

/// Corps de création d'un profil (nom et [`PROFIL`]).
pub fn corps_profil(nom: &str) -> String {
    format!(r#"{{"nom":"{nom}","profil":{PROFIL}}}"#)
}

/// Relevé de test lisible par [`PROFIL`], propre à `marque` (deux marques, deux fichiers).
pub fn releve(marque: &str) -> String {
    format!(
        "Date;Libellé;Montant;Solde\r\n\
         02/01/2026;Versement espèces {marque};150000,00;150000,00\r\n\
         03/01/2026;=FRAIS TENUE;-2500,00;147500,00\r\n\
         05/01/2026;Virement reçu;12345,50;159845,50\r\n"
    )
}

/// Corps d'exemple des mutations du module `releves` (`commun::serveur::corps_de`). Le dépôt
/// est un relevé fixe : le premier dépôt d'un dossier le crée, les suivants le signalent en
/// doublon (200) ; il diffère du dépôt fixe de [`doter`].
pub fn corps(cas: api::serveur::routes::releves::Cas) -> &'static str {
    use api::serveur::routes::releves::Cas;
    use std::sync::OnceLock;
    static PROFIL_CORPS: OnceLock<String> = OnceLock::new();
    static DEPOT: OnceLock<String> = OnceLock::new();
    match cas {
        Cas::CreerProfil | Cas::VersionnerProfil => {
            PROFIL_CORPS.get_or_init(|| corps_profil("Banque de test"))
        }
        Cas::Deposer => DEPOT.get_or_init(|| releve("corps d'exemple")),
        Cas::ListerProfils
        | Cas::LireProfil
        | Cas::ListerDepots
        | Cas::LireDepot
        | Cas::Analyser => "",
    }
}

/// Forme canonique stockée du profil [`PROFIL`] (celle que produit l'API).
pub fn definition() -> String {
    let brut: ProfilBrut = serde_json::from_str(PROFIL).unwrap();
    serde_json::to_string(&ProfilBrut::from(ProfilReleve::try_from(brut).unwrap())).unwrap()
}

/// Dote un dossier de test d'un profil (version 1) et d'un dépôt de relevé sur le compte de
/// trésorerie `compte` : objet dans le stockage partagé, lignes insérées par le
/// super-utilisateur. Rend (profil, dépôt).
pub async fn doter(
    admin: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    auteur: IdUtilisateur,
    compte: Uuid,
) -> (Uuid, Uuid) {
    let profil = Uuid::new_v4();
    let definition = definition();
    sqlx::query(
        "INSERT INTO profil_releve (cabinet_id, dossier_id, id, version, nom, definition, \
         empreinte, cree_par, cree_le) VALUES ($1, $2, $3, 1, 'Profil fixe', $4, $5, $6, now())",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(profil)
    .bind(&definition)
    .bind(Empreinte::calculer(definition.as_bytes()).octets().to_vec())
    .bind(auteur.uuid())
    .execute(admin)
    .await
    .expect("profil de test");

    let piece = piece_texte(admin, cabinet, dossier, auteur).await;
    let maintenant = time::OffsetDateTime::now_utc();
    let depot = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO depot_releve (cabinet_id, dossier_id, id, compte_tresorerie_id, piece_id, \
         depose_par, depose_le) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(depot)
    .bind(compte)
    .bind(piece)
    .bind(auteur.uuid())
    .bind(maintenant)
    .execute(admin)
    .await
    .expect("dépôt de test");
    (profil, depot)
}

/// Pièce externe `text/csv` réelle (relevé propre à la pièce), sans dépôt : objet dans le
/// stockage partagé, ligne insérée par le super-utilisateur. Rend son identifiant.
pub async fn piece_texte(
    admin: &PgPool,
    cabinet: IdCabinet,
    dossier: IdDossier,
    auteur: IdUtilisateur,
) -> Uuid {
    let piece = Uuid::new_v4();
    let contenu = releve(&piece.to_string()).into_bytes();
    let cle = CleObjet::nouvelle(cabinet, dossier);
    let version = Uuid::new_v4().to_string();
    StockageMemoire::partage()
        .0
        .lock()
        .expect("objets")
        .insert((cle.as_str().to_owned(), version.clone()), contenu.clone());
    let maintenant = time::OffsetDateTime::now_utc();
    sqlx::query(
        "INSERT INTO piece (cabinet_id, dossier_id, id, nature, type_contenu, taille, empreinte, \
         nom_origine, cle_objet, version_objet, retenue_jusqu_au, deposee_par, deposee_le) \
         VALUES ($1, $2, $3, 'externe', 'text/csv', $4, $5, 'releve.csv', $6, $7, $8, $9, $10)",
    )
    .bind(cabinet.uuid())
    .bind(dossier.uuid())
    .bind(piece)
    .bind(i64::try_from(contenu.len()).expect("taille"))
    .bind(Empreinte::calculer(&contenu).octets().to_vec())
    .bind(cle.as_str())
    .bind(version)
    .bind(api::pieces::ajouter_annees(maintenant, 11))
    .bind(auteur.uuid())
    .bind(maintenant)
    .execute(admin)
    .await
    .expect("pièce de relevé de test");
    piece
}
