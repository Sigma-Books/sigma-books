# 0018 — Passkeys : copie corrigée de `crypto-glue` en attendant sa version 0.2.3

- **Statut :** acceptée (2026-11-19, par le mainteneur)
- **Date :** 2026-11-19

## Contexte
L'ADR 0006 fait des passkeys la méthode principale d'authentification, et l'ADR 0012 en confie la vérification à `webauthn-rs` (MPL-2.0, audité par SUSE, Fondations §6.3).

`webauthn-rs` 0.6.2, la dernière version publiée, ne peut pas entrer dans le workspace :
- `webauthn-rs-core` dépend de `crypto-glue` 0.2.x (MPL-2.0, kanidm), qui exige `rsa = "0.10.0-rc.18"` avec la fonctionnalité `sha2` ;
- cette fonctionnalité n'existe plus dans `rsa` 0.10.0, la version stable utilisée par `integrite` pour l'horodatage (ADR 0010) ;
- cargo ne fait pas cohabiter une préversion et la version stable de la même série ;
- les préversions rc.18 et rc.19 tombent sous l'avis RUSTSEC-2023-0071 : `cargo deny` échoue.

Voies étudiées le 2026-11-19 :
- `webauthn-rs` 0.5 : dépend d'OpenSSL, exclu par les Fondations §6.3.
- Autres crates WebAuthn côté serveur : aucune n'est à la fois maintenue, auditée et compatible.
- Retirer RS256 : `webauthn-rs` ne permet pas de choisir les algorithmes, et `crypto-glue` dépend de `rsa` sans option.
- Binaire ou workspace séparé : exige quand même une exception RUSTSEC, avec un coût permanent.
- Vérification WebAuthn écrite par nous : environ 1 000 lignes de protocole de sécurité non auditées, contraire à l'esprit de « aucune implémentation maison ».
- Mot de passe et TOTP seuls : le TOTP ne résiste pas à l'hameçonnage par mandataire en temps réel.

En amont, la PR kanidm/crypto-glue#49 (« update rsa to 0.10 ») fait exactement la correction nécessaire, mais elle n'est ni fusionnée ni publiée.

## Décision
Décision du mainteneur du 2026-11-19 : garder les passkeys et **vendre dans le dépôt une copie corrigée** de `crypto-glue`.

- **Copie** : `vendor/crypto-glue/`, version 0.2.2 (tag v0.2.2, commit `e40f0c6`), branchée par `[patch.crates-io] crypto-glue = { path = "vendor/crypto-glue" }` dans le `Cargo.toml` du workspace. La copie n'est pas membre du workspace.
- **Correction minimale** :
  - `Cargo.toml` : `rsa = "0.10.0"` et la fonctionnalité `pkcs1v15` au lieu de `sha2` ;
  - `src/lib.rs` : alias de types (`RsaPrivateKey`, `RsaPublicKey`), soit environ 9 lignes en tout.
  
  Aucun autre fichier n'est modifié. `webauthn-rs` et `webauthn-rs-core` sont pris tels quels sur crates.io.
- **Licence (MPL-2.0, compatible avec l'AGPL-3.0)** : les fichiers restent sous MPL-2.0.
  - Le texte de la licence est ajouté dans la copie.
  - Les fichiers modifiés portent l'avis de l'Exhibit A et une mention de modification.
  - Un `README` de la copie décrit les changements.
  - La provenance est consignée dans `docs/provenance/crypto-glue.md` (règle absolue 11).
- **Aucune exception de CI** : `cargo deny` passe sans changement de `deny.toml`, car un chemin local n'est pas une source git. Une seule version de `rsa`, 0.10.0, est partagée avec `integrite`.
- **Surveillance manuelle** : `cargo deny` ne vérifie pas les avis de sécurité d'une crate locale. Jusqu'au retrait de la copie, toute PR qui touche à l'authentification, et la revue trimestrielle des avis (#145), vérifient les avis publiés pour `crypto-glue` et pour `webauthn-rs`.
- **Critère de sortie** : dès que `crypto-glue` 0.2.3, ou toute version 0.2.x qui dépend de `rsa` 0.10.0, est publiée, on supprime `vendor/crypto-glue/` et le `[patch]`, puis on lance `cargo update -p crypto-glue`. `webauthn-rs-core` accepte `crypto-glue ^0.2.1` : il n'y a pas besoin d'attendre une nouvelle version de `webauthn-rs`.
- **Pas d'intervention publique** du compte machine sur les dépôts amont (décision du mainteneur).
- **Mise en œuvre** avec la PR 8c (passkeys), dernière des PR d'authentification (8a sessions, 8b secours). L'issue #162 porte le critère de sortie.

## Conséquences
- Les passkeys restent la méthode principale (ADR 0006), avec la vérification d'une bibliothèque auditée.
- Le dépôt porte environ 3 650 lignes de code tiers copié sans modification, en dehors des 9 lignes corrigées.
- Le SBOM montre `crypto-glue` comme crate locale, sans identifiant crates.io : à expliquer dans la documentation de publication.
- Tests : la PR 8c reprend les tests de `webauthn-rs-core` qui touchent RS256 (par exemple Windows Hello TPM) et un parcours complet avec un authentificateur logiciel : enregistrement, authentification avec vérification de l'utilisateur, rejeu refusé.

## Alternatives écartées
Voir le contexte. En bref :
- **exception RUSTSEC** sur une préversion de `rsa` : elle affaiblit `cargo deny` (règle absolue 12) ;
- **autre crate** ou **implémentation maison** : ni l'une ni l'autre n'est auditée ;
- **binaire séparé** : coût permanent, sans lever l'exception ;
- **retirer les passkeys** : on perd la résistance à l'hameçonnage, et il faudrait remplacer les ADR 0006 et 0012.
