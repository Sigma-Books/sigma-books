# 0030 — Paramètres fiscaux par pays et codes de régime génériques

- **Statut :** proposée
- **Date :** 2026-11-26

## Contexte
Sigma Books tient des dossiers **SYSCOHADA révisé** pour toute la zone OHADA. L'ADR 0027 a posé le contexte du dossier (pays, devise de tenue, fuseau, langue, identifiants légaux) et renvoyé à cet ADR (point 5) les **paramètres fiscaux par pays**, leurs versions datées et les codes de régime. Décisions du mainteneur (2026-11-26) : socle SYSCOHADA commun ; **pack pays** = valeurs par défaut sourcées ; paramètres modifiables par dossier par **versions datées**, une version publiée n'étant jamais modifiée ; si Odoo et Tryton ont à peu près le même mécanisme, Sigma fait pareil (principe de l'ADR 0026) ; les textes ne priment que s'ils tranchent ; aucun contrôle plus strict que les moteurs sans raison documentée.

L'audit du 2026-11-26 a relevé que le multi-pays fiscal n'existe pas dans le code :
- **B1.** Le modèle `ParametresFiscaux` (pays, version, contenu) n'existe que dans la spec (§5.4, §23). `ReferentielTva::charger()` n'a aucun paramètre (`crates/fiscal-sn/src/tva.rs:529`) ; les deux fichiers de données sont embarqués par `include_str!` (`tva.rs:10-11`) sans version, date d'effet ni pays.
- **B2.** Un `OnceLock` global charge ce référentiel pour tous les dossiers (`crates/api/src/serveur/routes/brouillard/saisie.rs:34-41`) ; `api` dépend directement de `fiscal-sn` (`crates/api/Cargo.toml:32`). Seul `compta-core` passe par le trait `ReferentielFiscal`.
- **B3.** **La version de paramètres n'est jamais enregistrée** : la colonne existe (`0203_validation.sql:91`) et le contenu haché sait la porter (`parametres=`, `docs/integrite/format-v1.md:27`), mais `preparer` rebâtit chaque écriture sans version (`routes/brouillard/validation.rs:259-269`). La spec §5.4 impose pourtant « chaque écriture enregistre la version » ; l'ADR 0020 le notait comme limite.
- **B4.** Les codes de régime portent un **taux ou un article du CGI sénégalais** : `IMP18`, `IMP10`, `EXO361`, `EXO380`, `AUTOLIQ_REPFISC`, `EXO361-n`. Ils entrent dans le contenu haché, en ajout seul (ADR 0019 point 3, ADR 0007) : il faut les corriger **avant la première validation de production**.
- **B5.** La validité dans le temps n'est pas modélisée : `EXO380-e` expire le 31/12/2026 (`motifs-exoneration.toml:9, 280`) et rien ne l'applique.
- **B6.** La fourchette de pages du CGI sénégalais est en dur (`tva.rs:14-15`) ; deux énumérations sont propres au Sénégal (`Deduction::NonSaufChevauchement`, `ExigibiliteCgi::{Trimestrielle, MoisSuivant}` : art. 363-4 et 387-1).
- **B7.** La rétention des pièces cite l'art. 637 du CGI sénégalais en dur (`crates/api/src/pieces/mod.rs:34`, `RETENTION_MIN_ANS = 10`).
- **B8.** Taux et seuils : l'étiquette ne couvre que la TVA.

### Ce que font les moteurs de référence (lecture réelle, 2026-11-26)
- **Socle commun et paquet pays.** Odoo : `l10n_sn` hérite de `syscohada` (`l10n_sn/models/template_sn.py:8-14`) et apporte ses taxes (`l10n_sn/data/template/account.tax-sn.csv`, ligne `tva_sale_18`, montant `18.0`) ; la taxe est ensuite **copiée par société** (`account_tax.py:33`, `company_id` obligatoire). Tryton : `account_syscohada` ne livre que le plan (`account_syscohada/tryton.cfg:3-9`, aucun fichier de taxes) ; les taxes sont dans les modules de pays (`account_be/tax_be.xml`) et sont par société (`account/tax.py:132`). **Même mécanisme : socle commun + valeurs par défaut de pays, copiées dans l'entité tenue, que l'utilisateur peut ensuite modifier.**
- **Le taux est une donnée, jamais un code.** Dans les deux moteurs il est un champ de la taxe (Odoo `amount`, `account_tax.py:133` ; Tryton `rate` et `amount`, `account/tax.py`, classe `Tax`), jamais un identifiant de régime.
- **Validité dans le temps.** Tryton : `start_date` et `end_date` sur chaque taxe (`account/tax.py:757-758`), appliquées à la date du document (`_unit_compute`, `:1003-1006`, et `_reverse_rate_amount`, `:1032-1034`) ; la date est `accounting_date`, sinon `invoice_date`, sinon aujourd'hui (`account_invoice/invoice.py:1185-1195`). Odoo : **aucune date** sur la taxe, seulement un drapeau `active` (`account_tax.py:125`) : on archive l'ancienne taxe et on en crée une autre. **Les deux divergent** : Sigma suit Tryton (une date de début et de fin, appliquée à la date de l'opération), parce que l'archivage d'Odoo ne dit pas à quelle date une règle s'appliquait et que le contenu haché doit pouvoir la retrouver.
- **Aucun des deux ne versionne ni ne fige** la configuration fiscale utilisée par une écriture (Odoo et Tryton modifient la taxe en place). C'est l'écart assumé du point 3 : la règle absolue 5 et la spec §5.4 l'exigent.

## Décision

### 1. Une crate `fiscal`, un répertoire de données par pays et par version
La crate `fiscal-sn` est **renommée `fiscal`** (`crates/fiscal`) : le code (chargement, validation, sélection par date, registre) est commun, **seules les données varient** par pays. Les données d'un pays sont dans `crates/fiscal/data/pays/<xx>/<version>/` (code ISO 3166 en minuscules) : `version.toml` (en-tête : pays, version, date d'effet, date de fin, documents sources, taux nommés, rétention), `regimes-tva.toml`, `motifs-exoneration.toml` (les deux fichiers actuels, déplacés). Le **socle** commun est dans `crates/fiscal/data/socle/`.

*Tranché : une crate par pays (`fiscal-sn`, `fiscal-cm`…) est écartée.* Odoo et Tryton ont un module par pays parce que leur module porte aussi du code, des vues et des chargeurs ; le pack de Sigma est **uniquement de la donnée** (règle absolue 9 : les paramètres fiscaux vivent dans des fichiers de données qui citent leur source). Dix-sept crates identiques à leurs données près multiplieraient les dépendances de `api`, les lignes de CODEOWNERS et d'étiquettes, les revues de licence (`cargo deny`) et les points d'enregistrement. Ajouter un pays = ajouter un répertoire de données et **une ligne** dans le registre (point 2), sans autre code ni nouvelle dépendance.

Les fichiers d'une version publiée sont **immuables** : un test fige l'empreinte SHA-256 de chacun (liste `data/pays/<xx>/EMPREINTES`) ; la modifier exige de modifier ce fichier de contrôle, visible en revue. Une correction de la loi = un **nouveau** répertoire de version, jamais une retouche.

### 2. Le type `ParametresFiscaux` et le registre
```
ParametresFiscaux {
  pays, version, portee (pack | dossier), date_effet, date_fin?,
  source (documents : titre, édition, plage de pages du chapitre TVA),
  taux (nommés : normal, reduit… ; décimaux exacts, source),
  regimes, motifs, chevauchements, retention (années, source)
}
```
Le **registre** `Registre` associe un pays aux versions de son pack (`Registre::officiel()` : liste fermée en code, une ligne par pays, validée au démarrage ; le serveur refuse de démarrer si un pack est invalide). `api` ne nomme plus aucun pays ni aucune crate de pays : elle reçoit un registre, lit `dossier.pays` (ADR 0027) et lui demande la version applicable. **Le `OnceLock` global disparaît.** `compta-core` reste pur : il ne voit que le trait `ReferentielFiscal` (inchangé), que la vue d'une version à une date implémente (point 4). Un pays du pack de l'ADR 0027 **sans** version de paramètres est admis (il n'a simplement aucune étiquette possible : point 3).

La **valeur d'un pack ne remplace jamais le socle** : `data/socle/regimes-tva.toml` définit, pour chaque code générique, ce que `compta-core` suppose (exonéré ou non, ligne de taxe ou non, rôles de taux admis, exigibilités d'étiquette admises). Le pack apporte ce qui relève du **pays** (articles et pages, taux, droit à déduction et exigibilité légales, dates de validité, motifs) et ne peut ni créer un code ni changer une nature. Un régime propre à un pays que le socle ne couvre pas s'ajoute **au socle** (ajout seul, revue de l'équipe core), jamais au pack.

### 3. Version de paramètres : sélection par date, enregistrement, surcharge par dossier
- **Identifiant.** `<pays en minuscules>-<AAAA.N>` pour un pack (`sn-2025.1`) ; `<identifiant du pack de base>+d<N>` pour une version de dossier (`sn-2025.1+d2`). C'est la valeur de `VersionParametres` de `compta-core` (opaque) et de la clé `parametres=` du contenu haché.
- **Date de sélection : la date comptable de l'écriture** (celle de l'opération), non la date de validation, comme Tryton (`tax_date`). Une écriture de décembre validée en janvier suit les règles de décembre.
- **Versions applicables.** Dans une portée (pack du pays, ou versions du dossier), une version s'applique de sa `date_effet` jusqu'à la veille de la `date_effet` de la suivante, ou jusqu'à sa `date_fin` si elle est antérieure (abrogation sans successeur). Les versions d'un dossier **l'emportent** sur celles du pack aux dates où elles s'appliquent (comme la taxe d'une société d'Odoo ou de Tryton, qui ne suit pas les mises à jour du module de pays) ; une `date_fin` permet de revenir au pack. Deux versions d'une même portée ne partagent pas de `date_effet`.
- **Enregistrement.** La validation (unitaire, par lot) choisit la version par la date de l'écriture, **recontrôle les étiquettes contre elle**, l'écrit dans l'écriture (`VersionParametres`, contenu haché `parametres=`) et dans la colonne `ecriture_validee.version_parametres`. Le **brouillon** porte une colonne `version_parametres` (version appliquée à la dernière saisie, informative) ; la validation la recalcule, une version publiée entre-temps ne fait pas échouer la validation, elle fait seulement recontrôler les étiquettes. Une **contre-passation** reprend la version de son origine (l'étiquette est copiée telle quelle, ADR 0019 point 1 ; `compta-core` le fait déjà : `validation.rs:181`). La relecture d'une écriture validée (éditions, vérification, contre-passation) utilise **la version enregistrée**, jamais la version courante ; une écriture sans version (aucune donnée de production n'existe, mais le cas est lu) utilise la version applicable à sa date.
- **Aucune version applicable à la date** (écriture antérieure à la première version du pays, pays sans pack) : l'écriture **sans étiquette fiscale** se valide, sans version (contenu haché inchangé : ni Odoo ni Tryton ne refusent une écriture faute de taxe) ; une ligne **étiquetée** est refusée en saisie et en validation (422 `parametres_fiscaux_absents`), l'étiquette n'ayant aucun référentiel. La spec §5.4 se lit donc : chaque écriture enregistre la version **quand il en existe une**.
- **Surcharge par dossier = nouvelle version, jamais une modification.** L'administrateur du cabinet crée une version rattachée au dossier (`POST …/dossiers/{dossier}/parametres-fiscaux`, action Cedar `CreerVersionParametresFiscaux`, **ré-authentification**, audit chaîné avec l'identifiant et l'empreinte du contenu). Le contenu est un texte au **même format** que le pack, chargé par **le même validateur**, avec les mêmes contraintes (codes du socle seuls, source obligatoire pour chaque entrée : pour une version de dossier, la source est une référence libre de l'administrateur, p. ex. une convention ou une décision de l'administration). Table `parametres_fiscaux_dossier`, **ajout seul** (aucun `UPDATE` ni `DELETE`, déclencheur en seconde barrière), RLS forcée, unicité (dossier, date d'effet), empreinte SHA-256 du contenu. Aucune version de dossier ne peut modifier ou masquer ce qu'une écriture validée a déjà enregistré : l'écriture porte son identifiant, et la table est immuable.
- **Pas de contrôle plus strict que les moteurs** : aucun refus à la validation tiré du seul **contenu** d'une version (taux de TVA « incohérent » avec la loi, etc.) ; seuls sont refusés les contenus mal formés ou sans source.

### 4. Codes de régime génériques et validité dans le temps
**Le taux n'est plus dans le code.** Chaque régime taxable cite des **rôles de taux** du socle (`normal`, `reduit`) ; la version du pays donne leur valeur (`taux.normal = "18"`). L'étiquette de la ligne **recopie toujours le décimal exact** (ADR 0019 point 2 : jamais une référence), donc le contenu haché ne change pas de nature.

Codes **du socle** (ajout seul ; fenêtre de correction unique, close à la première validation en production, après quoi l'ADR 0019 point 3 s'applique de nouveau) :
| Ancien code | Nouveau code | Nature |
|---|---|---|
| `IMP18` | `TAXABLE_NORMAL` | taxable au taux normal |
| `IMP10` | `TAXABLE_REDUIT` | taxable au taux réduit |
| `IMP_ENC` | `TAXABLE_ENCAISSEMENT` | taxable, exigible à l'encaissement (taux normal ou réduit) |
| `EXO361` | `EXO_SANS_DEDUCTION` | exonéré sans droit à déduction |
| `EXO380` | `EXO_AVEC_DEDUCTION` | exonéré avec droit à déduction |
| `HORS_CHAMP` | `HORS_CHAMP` | hors champ (inchangé) |
| `AUTOLIQ_REPFISC` | `AUTOLIQUIDATION` | autoliquidation (taxe due par le destinataire) |
| `COLLECTE_NUM` | `COLLECTE_NUM` | collecte et reversement par un intermédiaire (inchangé) |
| `PRECOMPTE` | `PRECOMPTE` | précompte (inchangé) |
| `MARGE` | `MARGE` | régime de la marge (inchangé) |
| `SUSPENSION` | `SUSPENSION` | suspension (inchangé) |
| `REGUL` | `REGUL` | régularisation (inchangé) |
| `REMB_ANNUL` | `REMB_ANNUL` | remboursement ou annulation (inchangé) |

Motifs : le format haché lit un motif **préfixé du code de son régime** (`<régime>-<suffixe>`, ADR 0019 point 5, vérifié par `sigma-verifier` sans liste) ; cette règle **ne change pas** (formats v1 à v5 figés). `EXO361-n` devient `EXO_SANS_DEDUCTION-n` et `EXO380-x` devient `EXO_AVEC_DEDUCTION-x` : le **suffixe est une clé opaque propre au pack**, qui n'est ni un article ni un alinéa de loi (la source est dans la donnée) ; dans le pack du Sénégal elle reprend la numérotation actuelle pour que la correspondance soit mécanique. **Un code de motif garde son sens pour un pays dans toutes les versions de ce pays** (une version peut le clore par une date de fin, jamais le réaffecter ni le retirer) ; un test de continuité parcourt les versions de chaque pack. Le sens complet d'un motif est (pays, code), lu dans la version enregistrée par l'écriture.

**Validité.** Chaque régime et chaque motif d'une version porte `debut` et `fin` facultatifs (inclus). La vue de la version **à une date** (celle de l'écriture) ne contient que les entrées valides ce jour-là : un régime ou un motif échu est refusé à la saisie pour une date postérieure à sa fin, et reste lu pour les écritures antérieures. `EXO_AVEC_DEDUCTION-e` (ex-`EXO380-e`) reçoit `fin = 2026-12-31`, ce qui tranche la règle « à instruire » de l'ADR 0019.

### 5. Énumérations et constantes propres au Sénégal généralisées
- `Deduction::NonSaufChevauchement` devient `NonSaufExceptions` (non, sauf pour les motifs qui renvoient à un régime ouvrant droit : table `[[chevauchement]]`, dont le nom reste) ; `ExigibiliteCgi` devient `ExigibiliteLegale` ; `Trimestrielle` devient `DeclarationPeriodique` et `MoisSuivant` devient `DelaiApresEvenement` (la période et le délai restent dans `source_exigibilite` et `note`, avec leur article). Les noms de données TOML suivent.
- La fourchette de pages (157 à 210) devient la plage de pages du chapitre TVA du **document source**, déclarée par la version (`[[document]]`).
- **Rétention des pièces** : paramètre de la version, `retention.annees_min` et sa source (pour le Sénégal : 10, AUDCIF art. 24 et CGI art. 637, ADR 0015). Le socle fixe un **plancher OHADA de 10 ans** (AUDCIF art. 24, déjà retenu par l'ADR 0015) qu'aucune version ne peut baisser ; le réglage d'instance `SIGMA_PIECES_RETENTION_ANS` ne peut que **relever** la durée du pack. Elle est lue à la date du dépôt, pour le pays du dossier.
- **Identifiants légaux** (renvoi de l'ADR 0027 : l'alerte « NINEA : format », spec §8.4) : aucun format national n'est sourcé dans les références du dépôt. Le contrôle de forme est reconnu comme **donnée de version** future (par type d'identifiant, avec source), **non livré** ici ; l'alerte relève du lot L3 (Tiers), avec sa donnée sourcée.

### 6. Format haché
**Aucun changement du format canonique.** La clé `parametres=` existe depuis la v1 (facultative, placée après `origine=`) ; l'écrire dans toute écriture ne change ni la version du format (choisie par le contenu), ni les empreintes des écritures existantes (aucune n'en porte). À la différence de l'ADR 0027 (qui devait ajouter `devise-tenue=` et a pu le faire dans les versions 4 et 5, non publiées, mais qui ne pouvait pas toucher la v1 à v3 déjà produites), il n'y a **aucune évolution de format** à greffer ici ; les formats v4 et v5 ne sont donc pas touchés par cet ADR. Les **vecteurs publiés** (écriture v1 à v3, export v1 à v4) qui contiennent d'anciens codes (`IMP18`, `EXO361-7`…) **ne sont pas modifiés** : le vérificateur n'a pas de liste de codes, ces vecteurs restent valides comme exemples de format. Les nouveaux codes entrent dans les tests de l'API et de la crate `fiscal`.

### 7. Hors périmètre
- Le **contenu fiscal d'un deuxième pays** : seul le pack du Sénégal est rempli. Un **pack factice de test** (taux et régimes différents, valeurs inventées signalées comme telles, jamais publié dans `data/`) prouve qu'un autre pays se charge et s'applique **sans toucher au socle ni au code** ; il vit dans les tests.
- Retenues à la source, taxes sur les transferts d'argent, prélèvement sur paiements marchands, coefficients du dégressif et autres seuils (spec §5.4, B8) : le type `ParametresFiscaux` s'étend par de nouveaux fichiers de version (ajout, non rupture) quand leur lot les instruira (L5 et suivants).
- L'**interface** est traitée par un autre chantier ; les changements de contrat (codes, route de lecture des paramètres du dossier) sont listés dans la PR d'implémentation.

## Conséquences
- Un dossier applique les paramètres de son pays, à la date de chaque écriture ; l'écriture les enregistre et les protège dans la chaîne.
- Ajouter un pays ou une loi de finances = un répertoire de données sourcées et une ligne de registre, sans code.
- Les anciens codes sont **retirés, non aliasés** (aucune donnée de production). Toute base de développement existante est à recréer ou à corriger à la main ; les tests, vecteurs de l'API et seeds suivent.
- `crates/fiscal-sn` disparaît ; `.github/CODEOWNERS` (ligne `/crates/fiscal-sn/`) et l'étiquette `zone:fiscal-sn` sont à renommer par le mainteneur (le défaut `*` couvre la zone entre-temps). `docs/fondations.md` et la spec (§0, §5.4, §23) restent à mettre à jour séparément, comme le dit l'ADR 0027.
- Écart assumé vis-à-vis d'Odoo et de Tryton : leurs taxes sont modifiées en place ; ici une version n'est jamais modifiée (règle absolue 5, spec §5.4). Une version de dossier l'emporte sur le pack sans le suivre, comme leur copie par société.
- Limite : un dossier dont l'administrateur a créé une version de dossier ne suit plus les versions suivantes du pack tant qu'il n'a pas posé de date de fin ; l'interface devra le signaler.

## Alternatives écartées
- **Une crate par pays** : voir le point 1.
- **Alias des anciens codes** : aucune donnée de production ; garder deux noms pour un même régime perdrait l'injectivité du contenu haché (deux textes pour un même sens).
- **Sélection par la date de validation** : une écriture dont la date comptable est dans le passé changerait de règle selon le jour où on la valide.
- **Modifier une version publiée** (Odoo et Tryton modifient la taxe en place) : ôterait toute valeur à la version enregistrée.
- **Versions du dossier sous forme de correctifs** (diff sur le pack) : le contenu effectif d'une version dépendrait de l'état du pack ; une version de dossier est un contenu **complet**.
- **Codes de motif génériques par nature** (santé, éducation, export…) : cette classification n'est pas un fait des textes officiels ; elle serait une décision comptable non sourcée.
- **Pack qui peut créer des codes de régime** : `compta-core` suppose la nature des codes ; le socle seul la fixe.
- **Ajouter `version_parametres` à un nouveau format v6** : inutile, la clé existe.
