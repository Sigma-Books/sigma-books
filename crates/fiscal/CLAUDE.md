# crate `fiscal` — paramètres fiscaux par pays, versionnés

**Rôle :** socle commun, paquets de pays (données sourcées), chargement et validation, registre pays → versions, sélection par date. Taux de TVA, régimes, motifs d'exonération, rétention des pièces ; plus tard retenues, taxes sur les transferts, coefficients du dégressif.

**Spec :** §5.4, §26. **ADR :** 0030 (décisions), 0019 (étiquette), 0027 (pays).

## Règles propres
- **Socle** (`data/socle/`) : codes de régime génériques et ce que `compta-core` en suppose (exonéré, ligne de taxe, rôles de taux, exigibilités). Ajout seul ; un test fige la liste. Aucun code ne porte un taux ni un article de loi.
- **Paquet d'un pays** : `data/pays/<xx>/<version>/` (`version.toml`, `regimes-tva.toml`, `motifs-exoneration.toml`). Il ne crée aucun code, ne change aucune nature ; il donne taux, articles, pages, déduction, exigibilité, validité, motifs, rétention.
- **Une version publiée est immuable** : `data/pays/<xx>/EMPREINTES` fige le SHA-256 de chaque fichier (test). Une loi nouvelle = un nouveau répertoire de version + sa ligne d'empreintes + une ligne dans `src/officiel.rs`. Régénérer `EMPREINTES` pour une version déjà publiée est interdit.
- **Aucune valeur sans source** (article, page, document). Une valeur non sourcée reste absente et le cas est signalé (`non_trouve`).
- Un code de motif garde son sens pour un pays dans toutes ses versions (test de continuité) ; une `fin` le clôt.
- Les points de §26 de la spec ne sont pas codés tant qu'ils ne sont pas validés : prévoir le paramètre, pas la valeur.
- Une écriture mémorise la version utilisée (identifiant `sn-2025.1`, ou `sn-2025.1+d2` pour une version de dossier) : ne jamais modifier une version publiée.
- Les paquets de **test** (pays factice `ZZ`) vivent dans `tests/fixtures/`, jamais dans `data/`.
