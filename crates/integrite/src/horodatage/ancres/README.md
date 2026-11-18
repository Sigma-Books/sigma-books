# Ancres de confiance de l'horodatage (ADR 0010)

Certificats racines **publics** des autorités d'horodatage retenues, intégrés à `sigma-verifier`. Ce ne sont pas des secrets : ils ne contiennent qu'une clé publique.

| Fichier | Sujet | Validité | Empreinte SHA-256 du certificat |
|---|---|---|---|
| `digicert-trusted-root-g4.der` | DigiCert Trusted Root G4 | 2013-08-01 → 2038-01-15 | `552f7bdcf1a7af9e6ce672017f4f12abf77240c78e761ac203d1d9d20ac89988` |
| `sectigo-public-time-stamping-root-r46.der` | Sectigo Public Time Stamping Root R46 | 2021-03-22 → 2046-03-21 | `4941b001b8a97e961b7817c9d9e960ec4b056bfc915a8c1aabf6ef6b3ac046a5` |

## Origine (2026-11-18)
- DigiCert : `https://cacerts.digicert.com/DigiCertTrustedRootG4.crt` (certificat DER, auto-signé).
- Sectigo : `http://crt.sectigo.com/SectigoPublicTimeStampingRootR46.p7c` (paquet PKCS#7 ; seul le certificat auto-signé a été extrait).

**Contre-vérification** : la clé publique de chaque racine est identique à celle de la version contresignée de la même racine, présente dans les jetons réels obtenus le même jour auprès de `timestamp.digicert.com` et `timestamp.sectigo.com` (SHA-256 de la clé : `89d6e39d…f4c004` pour DigiCert, `3db6d7b6…8e9729` pour Sectigo).

## Remplacement
Une racine expirée ou révoquée est remplacée par un nouvel ADR, avec la même contre-vérification. Les jetons déjà émis restent vérifiables tant que leur racine reste dans la liste : la validité des certificats est contrôlée à la date du jeton.
