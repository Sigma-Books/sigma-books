# Fuzzing de `integrite`

- Cible `export` : l'analyseur d'export de `sigma-verifier` (v1 à v5, jetons, écritures canoniques v2 à v4 avec étiquette fiscale et devise de tenue compris) reçoit
  des octets quelconques (menace F7.5 du modèle de menaces). Elle vérifie qu'aucune entrée ne
  provoque de panique et qu'une écriture acceptée est exactement sa forme canonique.
- Cible `export_audit` : l'analyseur d'export d'audit (`docs/integrite/audit-v1.md`) et celui d'une
  entrée d'audit reçoivent des octets quelconques (F7.3, F7.5). Aucune panique, et une entrée
  acceptée est exactement sa forme canonique.
- Cible `jeton` : l'analyse ASN.1 d'un jeton et d'une réponse RFC 3161 (CMS, X.509) reçoit des
  octets quelconques. Aucune panique, et aucun jeton n'est accepté pour un témoin jamais signé.

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd crates/integrite
mkdir -p fuzz/corpus/export && cp tests/vecteurs/export-v*.txt tests/vecteurs/v4-rejets/*.txt fuzz/seeds/ecriture/*.txt fuzz/corpus/export/
cargo +nightly fuzz run export -- -max_total_time=600
mkdir -p fuzz/corpus/export_audit && cp tests/vecteurs/audit-v1/*.txt fuzz/corpus/export_audit/
cargo +nightly fuzz run export_audit -- -max_total_time=600
mkdir -p fuzz/corpus/jeton && cp tests/vecteurs/horodatage/*.ts[rt] fuzz/corpus/jeton/
cargo +nightly fuzz run jeton -- -max_total_time=600
```

Les semences `fuzz/seeds/ecriture/` sont des écritures canoniques v3 et v4 seules (étiquette fiscale, date de valeur comptable, devise de tenue CDF avec une ligne en devise ; la commande `cp tests/vecteurs/export-v*.txt` copie aussi `export-v5-reference.txt`) : la cible `export` les lit aussi comme une écriture isolée.

Le corpus et les plantages (`corpus/`, `artifacts/`) ne sont pas versionnés. Un plantage
trouvé devient un test de non-régression dans `tests/`. L'exécution continue en CI relève du
lot L9 (`docs/lots/L1.md`).
