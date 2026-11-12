# crate `fiscal-sn` — paramètres fiscaux du Sénégal

**Rôle :** paramètres fiscaux **versionnés** : taux de TVA, comptes de TVA par nature, retenues à la source, taxes sur les transferts d'argent, prélèvement sur paiements marchands, coefficients du dégressif, etc.

**Spec :** §5.4, §26.

## Règles propres
- Une version de paramètres = une date d'effet + la liste des textes sources (CGI consolidé DGID janvier 2025 + loi n° 2025-17 du 27/09/2025, puis lois de finances suivantes).
- **Aucune valeur sans source** (article du CGI, loi, page). Une valeur non sourcée reste absente et le cas est signalé.
- Les points listés en §26 de la spec ne sont pas codés tant qu'ils ne sont pas validés : prévoir le paramètre, pas la valeur.
- Une écriture mémorise la version de paramètres utilisée : ne jamais modifier une version publiée, en créer une nouvelle.
