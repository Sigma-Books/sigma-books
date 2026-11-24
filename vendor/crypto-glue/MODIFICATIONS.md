# Modifications apportées par Sigma Books

Copie de `crypto-glue` 0.2.2 (kanidm, MPL-2.0, https://github.com/kanidm/crypto-glue/), prise sur crates.io, fichiers conservés sauf ci-dessous (ADR 0018 du dépôt Sigma Books).

- `Cargo.toml` : `rsa = "0.10.0"` avec la fonctionnalité `pkcs1v15`, au lieu de `0.10.0-rc.18` avec `sha2` (fonctionnalité disparue de la version stable ; les préversions sont visées par RUSTSEC-2023-0071).
- `src/lib.rs` : ligne `use rsa::{…}` du module `rsa` (noms `PrivateKey` et `PublicKey` de `rsa` 0.10.0, `RsaPrivateKey` et `RsaPublicKey` étant dépréciés) et avis de modification en tête.
- Ajoutés : `LICENSE` (texte officiel de la MPL-2.0, https://www.mozilla.org/media/MPL/2.0/index.txt) et ce fichier. `Cargo.lock` retiré.

À retirer dès la publication de `crypto-glue` 0.2.3 (issue #162). Correction amont : kanidm/crypto-glue#49.
