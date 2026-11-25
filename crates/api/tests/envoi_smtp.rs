//! Transport SMTP (PR 9b ; ADR 0012, 0024 ; F12.1) : **TLS obligatoire**. Un faux relais qui
//! n'annonce pas `STARTTLS` (ou qui parle en clair là où le TLS est exigé) ne reçoit **rien** :
//! ni authentification, ni adresse, ni message. Sans base de données.
// Fichier de test : les `expect`/`unwrap` y sont voulus.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::envoi::config::{ConfigurationSmtp, Identifiants, ModeTls};
use api::envoi::modele::invitation;
use api::envoi::transport::{ErreurEnvoi, Transport, TransportSmtp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

/// Faux relais en clair : annonce `250 8BITMIME` mais pas `STARTTLS`, et enregistre ce qu'il lit.
async fn faux_relais() -> (u16, Arc<Mutex<Vec<u8>>>) {
    let ecouteur = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = ecouteur.local_addr().unwrap().port();
    let recu = Arc::new(Mutex::new(Vec::new()));
    let journal = Arc::clone(&recu);
    tokio::spawn(async move {
        while let Ok((mut flux, _)) = ecouteur.accept().await {
            let journal = Arc::clone(&journal);
            tokio::spawn(async move {
                let _ = flux.write_all(b"220 faux.relais ESMTP\r\n").await;
                let mut tampon = [0u8; 1024];
                loop {
                    let lu =
                        tokio::time::timeout(Duration::from_secs(3), flux.read(&mut tampon)).await;
                    let Ok(Ok(n)) = lu else { return };
                    if n == 0 {
                        return;
                    }
                    journal.lock().unwrap().extend_from_slice(&tampon[..n]);
                    let texte = String::from_utf8_lossy(&tampon[..n]).to_ascii_uppercase();
                    let reponse: &[u8] = if texte.starts_with("EHLO") || texte.starts_with("HELO") {
                        b"250-faux.relais\r\n250 8BITMIME\r\n"
                    } else if texte.starts_with("QUIT") {
                        let _ = flux.write_all(b"221 au revoir\r\n").await;
                        return;
                    } else {
                        b"250 ok\r\n"
                    };
                    let _ = flux.write_all(reponse).await;
                }
            });
        }
    });
    (port, recu)
}

fn configuration(port: u16, tls: ModeTls, avec_identifiants: bool) -> ConfigurationSmtp {
    ConfigurationSmtp {
        hote: "127.0.0.1".to_owned(),
        port,
        tls,
        identifiants: avec_identifiants.then(|| Identifiants {
            utilisateur: "envoi".to_owned(),
            mot_de_passe: Zeroizing::new("valeur-de-test-jetable".to_owned()),
        }),
        expediteur: "noreply@exemple.sn".to_owned(),
        nom_expediteur: "Sigma Books".to_owned(),
        nom_helo: Some("sigma.exemple.sn".to_owned()),
        delai: Duration::from_secs(5),
    }
}

fn message() -> api::envoi::modele::Message {
    invitation(
        "awa@cabinet.example",
        "Awa Diop",
        "https://compta.exemple.sn/lien#AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    )
    .unwrap()
}

fn rien_de_sensible(recu: &[u8]) {
    let texte = String::from_utf8_lossy(recu).to_ascii_uppercase();
    for interdit in [
        "AUTH",
        "MAIL FROM",
        "RCPT TO",
        "DATA",
        "AWA@CABINET",
        "LIEN#",
        "VALEUR-DE-TEST",
    ] {
        assert!(
            !texte.contains(interdit),
            "{interdit} reçu en clair : {texte}"
        );
    }
}

#[tokio::test]
async fn un_relais_sans_starttls_ne_recoit_rien() {
    let (port, recu) = faux_relais().await;
    for identifiants in [false, true] {
        let transport =
            TransportSmtp::new(&configuration(port, ModeTls::Starttls, identifiants)).unwrap();
        let resultat = transport.envoyer(&message()).await;
        assert_eq!(
            resultat,
            Err(ErreurEnvoi::Tls),
            "identifiants : {identifiants}"
        );
    }
    let recu = recu.lock().unwrap().clone();
    // Le relais a vu un EHLO, rien d'autre de sensible.
    assert!(
        String::from_utf8_lossy(&recu)
            .to_ascii_uppercase()
            .contains("EHLO")
    );
    rien_de_sensible(&recu);
}

#[tokio::test]
async fn le_tls_implicite_n_envoie_rien_en_clair() {
    let (port, recu) = faux_relais().await;
    let transport = TransportSmtp::new(&configuration(port, ModeTls::Implicite, true)).unwrap();
    let resultat = transport.envoyer(&message()).await;
    assert!(resultat.is_err(), "{resultat:?}");
    // Le TLS échoue avant tout échange SMTP : seules des octets de poignée de main ont circulé.
    rien_de_sensible(&recu.lock().unwrap());
}

#[tokio::test]
async fn un_relais_injoignable_est_un_echec_provisoire_sans_texte() {
    // Port fermé : on réserve un port puis on le libère.
    let port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let transport = TransportSmtp::new(&configuration(port, ModeTls::Starttls, false)).unwrap();
    let resultat = transport.envoyer(&message()).await;
    assert!(
        matches!(resultat, Err(ErreurEnvoi::Temporaire | ErreurEnvoi::Tls)),
        "{resultat:?}"
    );
    // L'erreur ne porte que son code stable.
    let texte = format!("{:?}{}", resultat, resultat.unwrap_err());
    assert!(
        !texte.contains("127.0.0.1") && !texte.contains("awa@"),
        "{texte}"
    );
}

#[test]
fn le_debug_du_transport_et_de_la_configuration_ne_montre_aucun_secret() {
    let config = configuration(2525, ModeTls::Starttls, true);
    let texte = format!("{config:?} {:?}", TransportSmtp::new(&config).unwrap());
    assert!(!texte.contains("valeur-de-test-jetable"), "{texte}");
}
