# 0027 — Pays, devise de tenue et paramètres du dossier

- **Statut :** proposée
- **Date :** 2026-11-26

## Contexte
Sigma Books tient des dossiers **SYSCOHADA révisé**, référentiel commun aux dix-sept États de l'OHADA. Le produit était pensé pour le Sénégal seul (spec §0 : « Pays v1 : Sénégal. Le modèle est multi-pays dès la conception »), et l'audit du 2026-11-26 a relevé que ce « multi-pays » n'existe pas dans le code :

- **A1.** La table `dossier` ne porte que `nom` (`crates/api/migrations/0001_organisation.sql`). La spec §23 prévoit « forme juridique, NINEA, RCCM, pays, devise, paramètres ». L'API ne rend que `{id, nom}` (`web/src/lib/api/types.ts`, type `Dossier`). Les dates du jour sont calculées en UTC (`validation.rs`, `lot.rs`, `contre_passation.rs`, `organisation.rs`, `editions.rs`).
- **A2.** La devise de tenue est figée à XOF : `compta-core` (`Devise::XOF`, `arrondi_xof`, `Ligne::en_xof`, `DesequilibreXof`, `PasEnXof`), `integrite` (format canonique : `montant=` « XOF », `devise=` « autre que XOF »), deux contraintes `CHECK devise <> 'XOF'` (migrations `0202` et `0203`), la saisie et la validation de l'API (`Montant::parse(…, Devise::XOF)`), la spec §5.5 (« Devise de tenue : XOF »).

Or la zone n'est pas monétairement uniforme : XOF (UEMOA : Bénin, Burkina Faso, Côte d'Ivoire, Guinée-Bissau, Mali, Niger, Sénégal, Togo), XAF (CEMAC : Cameroun, Centrafrique, Congo, Gabon, Guinée équatoriale, Tchad), KMF (Comores), GNF (Guinée), CDF (RDC, **deux décimales**). ISO 4217 (liste en vigueur, colonne « minor unit », recoupée le 2026-11-26) : XOF, XAF, KMF, GNF à 0 ; CDF, EUR, USD à 2.

Décisions du mainteneur (2026-11-26) : socle SYSCOHADA commun ; **pack pays** = valeurs par défaut sourcées ; paramètres modifiables par dossier, par versions datées, sans jamais modifier une version publiée ; **si Odoo et Tryton ont à peu près le même mécanisme, Sigma fait pareil** (principe de l'ADR 0026).

### Ce que font les moteurs de référence (lecture réelle, 2026-11-26)
- **Socle commun et pays** : Odoo a un module `l10n_syscohada` pour les dix-sept États (`l10n_syscohada/__manifest__.py`, description) et un module par pays qui en hérite : `l10n_sn/models/template_sn.py:8-14` (`'parent': 'syscohada'`) et `:16-26` (pays fiscal `base.sn`, taxes par défaut). Tryton a `account_syscohada` (plan commun) et des modules de pays à part. **Même mécanisme : un socle commun, un paquet par pays pour les valeurs par défaut.**
- **Devise de la société** : Odoo `company.currency_id` ; changement refusé s'il existe déjà des écritures (`account/models/company.py:755-759`) ; ses lignes sont arrondies par la devise (`account/models/account_move_line.py:783, 878` : `currency.round`), dont les décimales sont un réglage de la devise, non réductible une fois la devise utilisée (`account/models/res_currency.py:25-31`). Tryton `company.currency`, obligatoire (`modules/company/company.py:64-65`) ; `currency.rounding` et `currency.digits` sont des champs de la devise (`modules/currency/currency.py:56-66`), `round` s'y applique (`:152-166`). **Même mécanisme : la devise de tenue et son échelle viennent de données, jamais d'une constante, et ne changent plus après usage.** Écart assumé : Tryton arrondit au pair par défaut (`currency.py:152`) ; Sigma garde l'arrondi « demi éloigné de zéro » de la spec §6.6.
- **Fuseau** : Tryton `company.timezone`, « utilisé pour calculer la date du jour » (`company.py:66-67`), choisi dans la base IANA (`:29`). Odoo porte le fuseau sur l'utilisateur. Sigma suit Tryton, par dossier : une écriture se date dans le fuseau de l'entité tenue, quel que soit celui du serveur ou de l'utilisateur.
- **Identifiants légaux** : Tryton a une liste de couples type/code (`modules/party/party.py:832-852`, `type` et `code`), dont les types proposés viennent de la configuration et des modules de pays (`modules/party/configuration.py:68-74`). Odoo a deux champs fixes, `vat` et `company_registry`, dont seul le gabarit de saisie dépend du pays (`account/models/company.py:279-280`). **Les deux moteurs divergent** ; le mainteneur a tranché pour la liste générique, qui couvre NINEA, RCCM, NIF, IFU, NIU, etc. sans colonne propre à un pays.

## Décision

### 1. Contexte du dossier
La table `dossier` gagne, **à la création et pour toujours** (aucun `UPDATE` accordé), les colonnes :
| Colonne | Contenu | Contrôle |
|---|---|---|
| `pays` | ISO 3166-1 alpha-2 | l'un des dix-sept États du pack pays |
| `devise_tenue` | ISO 4217 | présente dans la table des devises du pack |
| `decimales_tenue` | 0 à 4 | **copiée** à la création depuis la table des devises, jamais d'une constante : une retouche ultérieure de la table ne modifie pas un dossier existant (même raison que `res_currency.py:25-31`) |
| `fuseau` | nom IANA | présent dans la table des fuseaux du pack |
| `langue` | BCP 47 | `fr` en v1 (seule interface) ; forme bornée |
| `forme_juridique` | texte libre, facultatif | 1 à 100 caractères, sans caractère de contrôle (les formes diffèrent d'un pays à l'autre ; AUSCGIE seulement pour les sociétés commerciales) |

Les **identifiants légaux** sont une table à part, `dossier_identifiant_legal(cabinet_id, dossier_id, type, valeur)`, une valeur par type et par dossier, RLS forcée comme toute table de dossier, **ajout seul** en v1 (créés avec le dossier, aucune modification ni suppression exposée ; la correction d'un identifiant viendra avec sa route et son audit). Le **type** est un code du pack pays du dossier (`NINEA`, `RCCM`…) ; la **valeur** est un texte libre de 1 à 64 caractères sans caractère de contrôle : aucun format national n'est sourcé dans le dépôt, donc aucun n'est imposé (un contrôle de format sera une donnée du pack, avec sa source). **Aucune colonne propre à un pays.**

La devise de tenue, son échelle, le pays, la langue et le fuseau sont **immuables** : Odoo refuse déjà de changer la devise une fois des écritures présentes (`company.py:755-759`) ; Sigma va plus loin, car la devise entre dans le contenu haché (point 3). Changer de pays ou de devise = nouveau dossier, avec reprise. Fuseau, langue, forme juridique et identifiants pourront devenir modifiables par une PR dédiée avec son audit ; cet ADR ne l'ouvre pas.

### 2. Pack pays : valeurs par défaut
Le **pack pays** est un fichier de données sourcé, `crates/syscohada/data/pays.toml`, avec une table de devises `devises.toml` (règle absolue 9 : données du dépôt avec leur source). Pour chacun des dix-sept États : devise, fuseau principal, langue par défaut, types d'identifiants légaux (code, libellé, source). À la création, le corps de la requête donne le **pays** (obligatoire : un défaut silencieux fixerait une devise immuable) ; `devise_tenue`, `fuseau`, `langue` sont facultatifs et valent par défaut ceux du pays. Une valeur fournie est contrôlée contre les tables, pas contre le pays (un dossier peut tenir ses comptes en EUR).

Un pays dont les types d'identifiants propres ne sont pas sourcés n'en propose que deux, génériques et sourcés : `RCCM` (AUSCGIE art. 17, p. PDF 5 : numéro d'immatriculation au registre du commerce et du crédit mobilier sur tous les actes) et `AUTRE`. `SN` ajoute `NINEA` (CGI Sénégal consolidé janvier 2025, p. PDF 163 : le NINEA « constitue le numéro d'identification fiscale »). Les types NIF, IFU, NIU, NCC des autres pays entrent avec leur source, en ajoutant une ligne de données.

Le pack ne contient **aucun paramètre fiscal** : ceux-ci relèvent de l'ADR suivant (point 5).

### 3. Devise de tenue et format haché (`integrite`)
- `compta-core` reste **pur** : la devise de tenue et son échelle sont une valeur passée en paramètre (`DeviseTenue`), portée par la ligne et vérifiée par le contexte de validation. `en_xof` devient `en_devise_tenue`, `DesequilibreXof` devient `DesequilibreTenue`, `PasEnXof` devient `PasEnDeviseTenue`. Aucun alias de dépréciation.
- **Les hachages existants ne changent pas.** Les formats v1 à v3 de l'écriture et v1 à v4 de l'export restent figés, octet pour octet, et sont produits pour tout dossier dont la devise de tenue est XOF.
- **Un nouveau format est nécessaire** pour une autre devise. Le format actuel ne dit pas dans quelle devise sont les `montant=` : il suppose XOF, et `sigma-verifier`, autonome, refuserait `devise=XOF` ou arrondirait au franc la contre-valeur d'un dossier en CDF. Pire, sans mention de la devise dans le texte haché, les mêmes écritures d'un dossier XAF et d'un dossier XOF auraient la même empreinte : la devise ne serait pas protégée par la chaîne. Le format haché **porte donc la devise de tenue et son échelle** (`devise-tenue=`, `decimales-tenue=`) : **écriture v4**, **export v5** (`docs/integrite/format-v5.md`). Choix de la version par le contenu, comme aux ADR 0019 et 0020 : un dossier XOF garde v1, v2 ou v3, jamais v4.
- **Compatibilité** : un `sigma-verifier` antérieur refuse l'écriture v4 (« écriture v1 attendue ») : un échec explicite, jamais un succès silencieux. La version courante lit les formats v1 à v5. La base stocke le format (`1` à `4`, contrainte élargie par migration, jamais modifiée sur place).
- **Échelle des montants** (précise le point 6 de l'ADR 0020, dont le texte reste valable pour l'existant) : à la **saisie** (brouillard), un montant de ligne ayant plus de décimales que l'échelle de la devise de tenue est refusé (422), comme Tryton refuse un nombre hors `digits` ; la contre-valeur d'une ligne en devise est arrondie à cette échelle. `compta-core` n'impose aucune échelle à la **relecture** d'une ligne : les écritures déjà validées en XOF avec décimales restent lisibles. Le format v4 relu par le vérificateur exige, lui, l'échelle déclarée (il n'existe que pour les nouveaux dossiers).
- Contraintes SQL : `CHECK devise <> 'XOF'` (migrations `0202` et `0203`) est remplacée par un déclencheur qui compare la devise d'une ligne à la devise de tenue du dossier (une contrainte ne peut pas lire une autre table). Dossiers existants : XOF, `SN`, `Africa/Dakar`, `fr`.

### 4. L'API expose le contexte
`GET …/dossiers/{dossier}` rend `{id, nom, pays, devise, decimales, fuseau, langue, forme_juridique, identifiants_legaux:[{type, valeur}]}` ; la liste et la création rendent la même forme (sans les identifiants dans la liste). `GET /api/cabinets/{cabinet}/pays` rend le pack (pays, défauts, types d'identifiants), pour la page de création ; elle est décidée par l'action existante `CreerDossier`. La date du jour (validation, lot, contre-passation, éditions, alertes) est calculée **dans le fuseau du dossier** ; la création, qui n'a pas encore de dossier, utilise le fuseau du pays choisi. Le serveur ne lit pas la base de fuseaux de l'IANA : la table des fuseaux du pack donne le décalage fixe de chacun des fuseaux de la zone (aucun n'observe l'heure d'été, `tzdata` 2026f). Un pays à heure d'été exigerait un nouvel ADR et une bibliothèque de fuseaux.

### 5. Suite : paramètres fiscaux par pays
L'ADR suivant (rédigé séparément) fixe les **paramètres fiscaux par pays**, leurs versions datées modifiables par dossier sans jamais modifier une version publiée, et les **codes de régime génériques**. Il s'appuie sur le champ `pays` du dossier et sur le pack défini ici ; il ne change ni la devise ni les identifiants.

### 6. Ce qui reste hors du dossier
- Le **plan de comptes SYSCOHADA** est commun à tous les pays (`crates/syscohada`) ; un dossier en reçoit une copie, comme avant.
- Les **règles d'inaltérabilité** (ADR 0007, 0014, 0020, 0021 : écriture définitive, chaîne, numérotation sans trou, contre-passation) sont les mêmes pour tous les pays.
- `docs/specification.md` (§0, §5.5, §6.6, §23) et `docs/fondations.md` seront mis à jour séparément : ils disent encore « Devise de tenue : XOF ».

## Conséquences
- Un dossier est tenu dans la devise et l'échelle de son pays ; les dossiers existants gardent XOF sans que leurs empreintes changent.
- Le format canonique compte une version de plus ; `sigma-verifier` et `docs/integrite/` suivent. La devise de tenue est protégée par la chaîne.
- Une donnée de pack (devise, fuseau, type d'identifiant) s'ajoute par une ligne sourcée, sans code.
- Risque assumé : un montant XOF avec décimales qui était accepté au brouillard est désormais refusé à la saisie (échelle 0) ; les brouillons déjà saisis dans ce cas devront être corrigés avant leur validation.
- Limite : seuls les fuseaux sans heure d'été sont admis (point 4).

## Alternatives écartées
- **Devise de tenue modifiable** : changerait le sens de montants déjà hachés. Refusée (Odoo la refuse déjà après usage).
- **Colonnes `ninea` et `rccm`** (spec §23) : une colonne par pays, que le mainteneur a écartée ; la liste générique remplace Odoo (`vat`, `company_registry`) par le mécanisme de Tryton.
- **Garder les formats actuels** (devise implicite XOF) : le vérificateur ne peut pas contrôler un dossier d'une autre devise, et la devise n'est pas protégée par la chaîne (point 3).
- **Un nouveau format pour tous les dossiers, XOF compris** : ferait changer les empreintes existantes. Refusée.
- **Bibliothèque de fuseaux IANA** (jiff, chrono-tz) : dépendance lourde, inutile pour des fuseaux sans heure d'été ; à reprendre si la zone de produit sort de l'OHADA.
- **Un pays par défaut (`SN`) à la création** : fixerait en silence une devise immuable.
