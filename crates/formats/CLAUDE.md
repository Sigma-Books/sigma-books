# crate `formats` — formats d'échange

**Rôle :** lecture et écriture des formats externes : FEC (18 colonnes), profils d'import/export déclaratifs (Sage 100, Odoo, CSV/Excel générique), relevés bancaires CSV, relevés mobile money (Wave, Orange Money).

**Spec :** §7.4, §8.2, §18. 

## Règles propres
- **Toute entrée est hostile** : limites de taille, de nombre de lignes, d'encodage ; erreurs explicites, jamais de `panic`/`unwrap` sur une donnée d'entrée.
- Une **cible de fuzzing** (`cargo-fuzz`) par analyseur.
- Tests « golden files » : une entrée canonique donnée produit, octet pour octet, le fichier attendu pour chaque profil.
- Aucune dépendance à la base de données ni au réseau.
