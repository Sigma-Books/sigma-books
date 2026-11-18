# crate `integrite` — preuve d'inaltérabilité

**Rôle :** sérialisation canonique des écritures, chaîne de hachage SHA-256 par dossier, horodatage externe RFC 3161, **outil de vérification** en ligne de commande (binaire publiable seul, pour les auditeurs).

**Spec :** §6.4. **Fondations :** §3.6.

## Règles propres
- Sérialisation canonique **déterministe et documentée** (ordre des champs, format des montants et des dates, tri des lignes). Toute modification du format = nouvelle version de format, jamais un changement silencieux.
- Cryptographie : RustCrypto uniquement (`sha2`, `sha1`, `rsa`, `der`, `x509-cert`, `cms`, `x509-tsp`), aucune implémentation maison (ADR 0010).
- L'outil de vérification n'a besoin que d'un export : il ne se connecte à rien.
- Tests : une modification d'un seul octet d'une écriture doit être détectée ; vecteurs de test figés dans le dépôt.
