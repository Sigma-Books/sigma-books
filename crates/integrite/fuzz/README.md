# Fuzzing de `integrite`

Cible `export` : l'analyseur d'export de `sigma-verifier` reçoit des octets quelconques (menace
F7.5 du modèle de menaces). Elle vérifie qu'aucune entrée ne provoque de panique et qu'une
écriture acceptée est exactement sa forme canonique.

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd crates/integrite
mkdir -p fuzz/corpus/export && cp tests/vecteurs/export-v1.txt fuzz/corpus/export/
cargo +nightly fuzz run export -- -max_total_time=600
```

Le corpus et les plantages (`corpus/`, `artifacts/`) ne sont pas versionnés. Un plantage
trouvé devient un test de non-régression dans `tests/`. L'exécution continue en CI relève du
lot L9 (`docs/lots/L1.md`).
