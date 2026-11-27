# Fuzzing de `formats`

- Cible `releve_csv` : l'analyseur de relevés CSV reçoit des octets quelconques avec un profil
  engendré par les trois premiers octets (encodage, délimiteur, guillemet, en-tête, pied, format
  de date, séparateurs, signe inversé, colonnes par rang ou par titre) ; menace F11.4. Aucune
  panique, bornes respectées, et un relevé accepté et représentable se réécrit puis se relit à
  l'identique.
- Cible `profil_toml` : le chargeur de profil reçoit un TOML quelconque. Aucune panique, et un
  profil accepté se réécrit puis se relit à l'identique.
- Cible `motif` : un motif de règle de suggestion (source quelconque, bornes fixes) est compilé
  puis cherché dans un texte quelconque. Aucune panique ; une source au-delà de la borne est
  toujours refusée.

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd crates/formats
cargo +nightly fuzz run releve_csv fuzz/corpus/releve_csv fuzz/seeds/releve_csv -- -max_total_time=600
cargo +nightly fuzz run profil_toml fuzz/corpus/profil_toml fuzz/seeds/profil_toml -- -max_total_time=600
cargo +nightly fuzz run motif fuzz/corpus/motif fuzz/seeds/motif -- -max_total_time=600
```

Les semences `fuzz/seeds/` sont **synthétiques** (banques, clients, références et identifiants
inventés ; aucune donnée réelle) : relevés précédés de leurs trois octets de profil, profils des
fichiers de référence (`tests/releves/*.toml`), motifs précédés de leur longueur.

`fuzz/Cargo.lock` est **versionné** : la CI le veut complet et à jour (`cargo metadata
--locked`). Après un changement des dépendances de `formats`, le régénérer à partir de celui du
workspace, pour garder les versions contrôlées par `rust-deny` :
`cp ../../Cargo.lock fuzz/Cargo.lock && cargo +nightly metadata --format-version 1
--manifest-path fuzz/Cargo.toml > /dev/null`. `rust-deny` ne lit pas ce paquet hors workspace ;
avec `deny.toml`, seule la licence de `libfuzzer-sys` (`(MIT OR Apache-2.0) AND NCSA`, libFuzzer
de LLVM, outil de test jamais distribué) n'est pas dans la liste blanche, comme pour
`crates/integrite/fuzz`.

En CI, le job `rust-fuzz-smoke` de `.github/workflows/rust.yml` lance chaque cible 60 secondes,
seulement quand `crates/formats/` change (autorisation du mainteneur du 2026-11-27, décision D5
du plan L3, issue #255) ; il n'est pas requis. Les campagnes longues restent hors CI. Le corpus et
les plantages (`corpus/`, `artifacts/`) ne sont pas versionnés ; un plantage trouvé devient un
test de non-régression dans `tests/` ; en CI, il est conservé comme artefact `fuzz-plantages`.
