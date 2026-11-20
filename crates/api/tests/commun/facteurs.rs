//! Aides des tests de la méthode de secours (PR 8b) : secours et codes installés par le
//! super-utilisateur (chiffrés par la clé de test, comme le ferait l'enrôlement), appels de
//! connexion.

use api::base::IdUtilisateur;
use api::facteurs::codes;
use api::facteurs::mot_de_passe;
use api::facteurs::totp::SecretTotp;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::serveur::{ORIGINE, Reponse, appeler, cle_de_test};

/// Mot de passe des tests : 12 caractères ou plus, absent de la liste des mots de passe
/// compromis.
pub const MOT_DE_PASSE: &str = "cheval agrafe pile correcte";

/// Courriel d'un utilisateur de test (`commun::creer_utilisateur`).
pub fn courriel(utilisateur: IdUtilisateur) -> String {
    format!("{utilisateur}@exemple.invalid")
}

/// Installe une méthode de secours (mot de passe haché, secret TOTP chiffré par la clé de
/// test, aucun pas encore accepté) ; rend le secret.
pub async fn installer_secours(
    admin: &PgPool,
    utilisateur: IdUtilisateur,
    mot_de_passe: &str,
) -> SecretTotp {
    let secret = SecretTotp::tirer().expect("aléa");
    let cle = cle_de_test();
    let chiffre = cle
        .chiffrer_totp(utilisateur.uuid(), &secret)
        .expect("chiffrement");
    let chaine = mot_de_passe::hacher(mot_de_passe).expect("hachage");
    // Remplacement par suppression puis insertion : une mise à jour sans utilisateur en
    // contexte serait refusée par le déclencheur de garde de la connexion (migration 0100).
    sqlx::query("DELETE FROM facteur_secours WHERE utilisateur_id = $1")
        .bind(utilisateur.uuid())
        .execute(admin)
        .await
        .expect("secours retiré");
    sqlx::query(
        "INSERT INTO facteur_secours (utilisateur_id, mot_de_passe, totp_chiffre, totp_cle_id, \
                                      totp_dernier_pas, cree_le, modifie_le) \
         VALUES ($1, $2, $3, $4, 0, now(), now())",
    )
    .bind(utilisateur.uuid())
    .bind(chaine)
    .bind(chiffre)
    .bind(cle.identifiant().to_vec())
    .execute(admin)
    .await
    .expect("secours installé");
    secret
}

/// Installe une série de codes de récupération (l'ancienne est supprimée) ; rend leurs textes.
pub async fn installer_codes(admin: &PgPool, utilisateur: IdUtilisateur) -> Vec<String> {
    let serie = codes::tirer_serie().expect("aléa");
    sqlx::query("DELETE FROM code_recuperation WHERE utilisateur_id = $1")
        .bind(utilisateur.uuid())
        .execute(admin)
        .await
        .expect("suppression des codes");
    for code in &serie {
        sqlx::query(
            "INSERT INTO code_recuperation (id, utilisateur_id, empreinte, cree_le) \
             VALUES ($1, $2, $3, now())",
        )
        .bind(Uuid::new_v4())
        .bind(utilisateur.uuid())
        .bind(code.empreinte().to_vec())
        .execute(admin)
        .await
        .expect("code installé");
    }
    serie.iter().map(|c| c.texte().to_owned()).collect()
}

/// Code TOTP du secret à `instant`.
pub fn code(secret: &SecretTotp, instant: OffsetDateTime) -> String {
    secret.code_a(instant).expect("instant après 1970")
}

/// Décode une base32 RFC 4648 sans remplissage (secret rendu par la préparation).
pub fn base32_decoder(texte: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut sortie = Vec::new();
    let (mut tampon, mut bits) = (0u32, 0u32);
    for c in texte.bytes() {
        let valeur =
            u32::try_from(ALPHABET.iter().position(|a| *a == c).expect("base32")).expect("rang");
        tampon = (tampon << 5) | valeur;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            sortie.push(u8::try_from((tampon >> bits) & 0xff).expect("octet"));
        }
    }
    sortie
}

/// `POST` sans session sur une route de connexion, avec un corps JSON et un `User-Agent`
/// (l'appareil enregistré).
pub async fn connecter(routeur: &Router, chemin: &str, corps: &str, appareil: &str) -> Reponse {
    let requete = Request::post(chemin)
        .header("origin", ORIGINE)
        .header("sec-fetch-site", "same-origin")
        .header("x-sigma-csrf", "1")
        .header("content-type", "application/json")
        .header("user-agent", appareil)
        .body(Body::from(corps.to_owned()))
        .expect("requête");
    appeler(routeur, requete).await
}

/// Corps d'une connexion par le secours.
pub fn corps_secours(courriel: &str, mot_de_passe: &str, code: &str) -> String {
    serde_json::json!({ "courriel": courriel, "mot_de_passe": mot_de_passe, "code": code })
        .to_string()
}

/// Corps d'une connexion par un code de récupération.
pub fn corps_code(courriel: &str, code: &str) -> String {
    serde_json::json!({ "courriel": courriel, "code": code }).to_string()
}

/// Valeur de l'en-tête `Cookie` tirée du `Set-Cookie` d'une réponse.
pub fn cookie_de_reponse(reponse: &Reponse) -> String {
    reponse.entetes["set-cookie"]
        .to_str()
        .expect("ASCII")
        .split(';')
        .next()
        .expect("paire")
        .to_owned()
}
