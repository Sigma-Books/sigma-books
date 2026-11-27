# crate `formats` — formats d'échange

**Rôle :** lecture et écriture des formats externes : FEC (18 colonnes), profils d'import/export déclaratifs (Sage 100, Odoo, CSV/Excel générique), relevés bancaires CSV, relevés mobile money (Wave, Orange Money).

**Spec :** §7.4, §8.2, §9, §18. **ADR :** 0034 (relevés importés, §4, §6 à §8).

## Règles propres
- **Toute entrée est hostile** : limites de taille, de nombre de lignes, d'encodage ; erreurs explicites, jamais de `panic`/`unwrap` sur une donnée d'entrée.
- **Pur** : les analyseurs travaillent sur `&[u8]` ou `&str` ; aucune lecture de fichier, aucune base, aucun réseau, aucune horloge (une année absente n'est jamais complétée), aucune locale du serveur (dates et nombres sont lus par le profil).
- **Limites reçues de l'appelant** (`releve::Limites`, `motif::BornesMotif`) : leurs valeurs sont des paramètres de l'instance lus par `api`, jamais codées ici (ADR 0034 §6).
- Les erreurs ne portent que des positions (ligne, champ), jamais un libellé ni un montant (F2.11).
- Une **cible de fuzzing** (`cargo-fuzz`) par analyseur (`fuzz/`).
- Tests « golden files » : une entrée canonique donnée produit, octet pour octet, le fichier attendu pour chaque profil.
- Aucune dépendance à la base de données ni au réseau.

## Dépendances admises
`rust_decimal`, `thiserror`, `time`, `serde` (derive) et `toml` (profils), `sha2` (empreintes, RustCrypto), `encoding_rs` (jeux de caractères à un octet), `regex` (motifs en temps linéaire, taille bornée). Toute autre dépendance est justifiée dans la PR.
