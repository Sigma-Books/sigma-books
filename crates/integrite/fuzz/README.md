# Fuzzing de `integrite`

- Cible `export` : l'analyseur d'export de `sigma-verifier` (v1 et v2, jetons compris) reçoit
  des octets quelconques (menace F7.5 du modèle de menaces). Elle vérifie qu'aucune entrée ne
  provoque de panique et qu'une écriture acceptée est exactement sa forme canonique.
- Cible `jeton` : l'analyse ASN.1 d'un jeton et d'une réponse RFC 3161 (CMS, X.509) reçoit des
  octets quelconques. Aucune panique, et aucun jeton n'est accepté pour un témoin jamais signé.

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd crates/integrite
mkdir -p fuzz/corpus/export && cp tests/vecteurs/export-v*.txt fuzz/corpus/export/
cargo +nightly fuzz run export -- -max_total_time=600
mkdir -p fuzz/corpus/jeton && cp tests/vecteurs/horodatage/*.ts[rt] fuzz/corpus/jeton/
cargo +nightly fuzz run jeton -- -max_total_time=600
```

Le corpus et les plantages (`corpus/`, `artifacts/`) ne sont pas versionnés. Un plantage
trouvé devient un test de non-régression dans `tests/`. L'exécution continue en CI relève du
lot L9 (`docs/lots/L1.md`).
