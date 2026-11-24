//! RS256 par la copie corrigée de `crypto-glue` (ADR 0018) : `webauthn-rs-core` vérifie les
//! signatures RS256 (Windows Hello, certains authentificateurs TPM) par `crypto-glue`, que la
//! copie du dépôt branche sur `rsa` 0.10.0. Ce test éprouve cette voie de bout en bout : clé,
//! signature PKCS#1 v1.5 SHA-256 et vérification, sans base de données ni réseau. L'ES256 (la
//! voie des tests de bout en bout, `tests/passkeys.rs`) n'y passe pas.
// Fichier de test : les `expect` y sont voulus.
#![allow(clippy::expect_used)]

use crypto_glue::rsa::{MIN_BITS, RS256PublicKey, RS256SigningKey, RS256VerifyingKey, new_key};
use crypto_glue::traits::{SignatureEncoding, Signer, Verifier};

#[test]
fn une_signature_rs256_se_verifie_et_une_alteration_est_refusee() {
    let cle = new_key(MIN_BITS).expect("clé RSA de 2048 bits");
    let publique = RS256PublicKey::from(&cle);
    let signataire = RS256SigningKey::new(cle);
    let verificateur = RS256VerifyingKey::new(publique);
    let message = b"defi de test, jamais un secret";
    let signature = signataire.sign(message);
    assert!(verificateur.verify(message, &signature).is_ok());
    // Message altéré, signature altérée, signature d'un autre message : refusées.
    assert!(
        verificateur
            .verify(b"defi de test, autre", &signature)
            .is_err()
    );
    let mut octets = signature.to_bytes().to_vec();
    octets[0] ^= 0x01;
    let alteree = crypto_glue::rsa::pkcs1v15::Signature::try_from(octets.as_slice())
        .expect("longueur conservée");
    assert!(verificateur.verify(message, &alteree).is_err());
    // Un autre signataire (autre clé) ne se fait pas passer pour le premier.
    let autre = RS256SigningKey::new(new_key(MIN_BITS).expect("seconde clé"));
    assert!(verificateur.verify(message, &autre.sign(message)).is_err());
}
