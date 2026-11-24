# Reprise de `crypto-glue` (copie corrigée)

| Champ | Valeur |
|---|---|
| Projet | `crypto-glue` (kanidm) |
| Dépôt | https://github.com/kanidm/crypto-glue/ |
| Version | 0.2.2, archive publiée sur crates.io (tag v0.2.2, commit `e40f0c6`) |
| Licence | MPL-2.0 (compatible AGPL-3.0, ADR 0018 ; texte dans `vendor/crypto-glue/LICENSE`, SHA-256 `3f3d9e0024b1921b067d6f7f88deb4a60cbe7a78e76c64e3f1d7fc3b779b9d04`) |
| Auteurs / titulaires | William Brown et contributeurs du projet kanidm |
| Nature de la reprise | Copie en dépendance corrigée (`[patch.crates-io]`), fichiers sous MPL-2.0 |
| Destination | `vendor/crypto-glue/` |
| Modifications | `Cargo.toml` (`rsa` 0.10.0, `pkcs1v15`) et ligne `use` de `src/lib.rs` ; détail dans `vendor/crypto-glue/MODIFICATIONS.md` |
| Raison | `webauthn-rs-core` 0.6.2 exige `rsa` 0.10.0-rc.18, visée par RUSTSEC-2023-0071 (ADR 0018) |
| Sortie | Version 0.2.3 de `crypto-glue` publiée : supprimer `vendor/` et le `[patch]` (#162) |
| PR | PR 8c du lot L2 |

Les avis de sécurité de `crypto-glue` et de `webauthn-rs` sont vérifiés à chaque PR d'authentification (ADR 0018) : aucun avis publié à la date de cette PR (`cargo deny`, avis RustSec).
