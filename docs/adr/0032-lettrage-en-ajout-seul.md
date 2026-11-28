# 0032 — Lettrage en ajout seul : liens partiels, lettres, ruptures, écarts, règles automatiques, verrou par compte

- **Statut :** proposée
- **Date :** 2026-11-27

## Contexte
Le lot L3 livre le **lettrage** (spec §10.1) : rapprocher des lignes d'un même compte lettrable (tiers, 585, comptes d'attente) pour en constater le solde. Les deux moteurs de référence **mutent** pour cela : Tryton écrit `reconciliation` sur la ligne ; Odoo met à jour des champs stockés sur la ligne et **supprime** les liens au dé-lettrage. Sigma ne peut modifier ni une écriture validée ni son contenu haché (règle absolue 5, ADR 0007, formats v1 à v5 figés, `docs/integrite/`) : le lettrage s'écrit dans des **tables en ajout seul**, le dé-lettrage et toute correction sont des **lignes de rupture**, jamais des suppressions (principe P3 du plan L3). Cet ADR pose la structure, les conditions, la datation, les lettres, les écarts, les règles automatiques et l'**ordre des verrous** (compléments des ADR 0011 et 0020).

Décisions du mainteneur du 2026-11-27 (« suivez vos recommandations ») : **B1** liens partiels ; **B2** rupture en ajout seul, refus après clôture de l'exercice du lettrage. Provenance dite honnêtement dans chaque point. Références de moteurs **relues le 2026-11-27** (Tryton `f5354cc`, Odoo `fa175fb`, OCA `account-reconcile` `fa7be05` et `bank-statement-import` `058a9a8`, Bigcapital `1644e5b`).

### Ce que font les moteurs de référence (lecture réelle)
- **Structure.** Tryton : une `account.move.reconciliation` (numéro, date, `delegate_to`) regroupe des lignes qui portent son identifiant (`account/move.py:545-563`) ; **total seulement** : `check_lines` impose lignes valides, **même compte**, compte lettrable, **même tiers**, débit égal au crédit (`:646-690`) ; un reliquat passe par une écriture de **scission** et `delegate_to` (`account_receivable_rule/account.py:263-277`). Odoo : `account.partial.reconcile` lie **une ligne au débit et une au crédit** avec montant (`account/models/account_partial_reconcile.py:15-56`), le résiduel de la ligne est **calculé** à partir de ces liens (`account_move_line.py:818-882`, champ stocké) ; lettrage **partiel natif**.
- **Conditions.** Tryton (`:646-690`) : lignes valides, même compte, compte lettrable, même tiers, somme nulle (au sens de l'arrondi de la devise). Odoo (`account_move_line.py:2685-2710`) : ni déjà lettrée, ni annulée, même compte, compte lettrable (les comptes de liquidité aussi) ; **aucune occurrence de `partner`** : le tiers n'est pas contrôlé, le groupement par tiers est fait par l'interface OCA (`account_reconcile_oca/models/account_account_reconcile.py:50-98`). Tryton accepte des lignes d'écritures non comptabilisées (`:1630`, `posted &=`).
- **Période et verrou.** Aucun refus : Tryton exclut `reconciliation` du contrôle de modification d'une ligne comptabilisée (`:1074-1075`). Odoo ne refuse pas non plus, il **décale** à la fin du verrou la date des écritures de change ou de TVA sur encaissement qu'il doit contre-passer (`account_partial_reconcile.py:152-166`).
- **Dé-lettrage.** Tryton supprime la `Reconciliation` (`:2171-2181`) et **avertit** seulement si elle était déléguée ou contient une écriture d'écart (`:605-634`). Odoo supprime les liens, contre-passe les écritures de change générées et recalcule les numéros (`account_partial_reconcile.py:105-173`). Ni trace, ni contrôle de période.
- **Date du lettrage.** Tryton : « plus haute date des lignes » (`:555-557`, `:1683-1685`). Odoo : `max_date`, « utilisé pour déterminer à quelle date ce lettrage doit apparaître dans les rapports de créances et de dettes âgées » (`account_partial_reconcile.py:64-67`, `:85-90`) ; OCA lit ce champ (`account_financial_report/report/aged_partner_balance.py:99-141`). **Aucun moteur ne date par l'opération.**
- **Lettre.** Tryton : numéro tiré d'une **séquence par société** (`account/configuration.py:50`), trié par longueur puis texte (`move.py:583-585`). Odoo : « numéro de rapprochement » dérivé de l'identifiant du plus petit lien d'un **graphe connexe** de liens (`P…` ; identifiant du lettrage total sinon), recalculé et réécrit sur les lignes à chaque changement (`account_partial_reconcile.py:212-258`). **Aucun n'a des lettres alphabétiques par compte** : c'est la spec §10.1 (« code de lettrage par compte »).
- **Imputation.** Odoo impute dans l'ordre des lignes triées par **échéance sinon date** (`account_move_line.py:2567-2631`, tri `:2735-2745`) ; Tryton trie par `maturity_date or date` (`:1873-1874`) et `account_receivable_rule` impute le crédit d'un tiers sur les plus anciennes échéances d'abord (`account_receivable_rule/account.py:160-224`).
- **Écart.** Tryton : `WriteOff` nommé, journal de type « write-off » et **comptes débit et crédit configurés** (`move.py:1932-1955`), écriture générée (`:1690-1762`) et comptabilisée automatiquement si les lignes l'étaient (`:1652-1668`) ; sans écart configuré, le lettrage déséquilibré est refusé (`:1652-1657`). Odoo : les lignes d'écart viennent des modèles de lettrage ; la logique est dans OCA `account_reconcile_oca/models/account_reconcile_model.py:248-279` (reprise d'anciennes versions d'Odoo ; **le fichier de la base Odoo cloné ne la contient pas**, correction du plan qui l'attribuait à `odoo/addons/account/models/account_reconcile_model.py`).
- **Automatique.** Tryton : `find_best_reconciliation` est une **approximation gloutonne** (`:1828-1871`), `reconcile_automatic` l'**applique directement** par (compte, tiers, devise) (`:1877-1904`). OCA : correspondance par référence (`ILIKE`), même signe, même tiers si connu, **candidat unique** (`COUNT(*) = 1`, `account_bank_statement_line.py:980-1060`) ; une référence **vide** (`''`) correspondrait à tout. La **combinaison** de montants n'existe dans aucun moteur.
- **Texte.** Aucun emploi comptable du mot « lettrage » dans l'AUDCIF ni le Guide : le texte ne tranche rien ici. AUDCIF fiche 40, p. PDF 381 (relue) : sous-compte individuel « en principe ».

## Décision

### 1. Structure : liens partiels en ajout seul, résiduel calculé (B1)
*Provenance : **vérifiée contre les moteurs** : liens partiels à la Odoo, conditions de Tryton ; la spec §10.1 demande le lettrage « total ou partiel ». Tryton obligerait à créer des écritures de scission.*
- `lettrage (id, compte, lettre, date, auteur, origine)` : une **opération** (origine : `manuelle`, `automatique`, `proposition_validee`) ; unicité (dossier, compte, lettre).
- `lettrage_lien (lettrage, ligne_debit, ligne_credit, montant)` : un lien **entre une ligne au débit et une ligne au crédit** du compte (une ligne = écriture validée et rang de ligne), `montant > 0` à l'échelle de la devise de tenue (ADR 0027). Chaque lien impute le même montant des deux côtés : **débit imputé = crédit imputé par construction**, sans tolérance (décimal exact ; Bigcapital compare à 0,01 en flottant, `BankingMatching/commands/MatchTransactions.ts:111-118` : non repris).
- `lettrage_rupture (lettrage, date, auteur, motif)` : au plus une par lettrage.
- **Résiduel** d'une ligne = |montant| moins la somme des liens **non rompus** qui la citent ; **jamais écrit sur la ligne**, jamais stocké : calculé par agrégat (index sur les deux colonnes de ligne). Un état dérivé mutable (`D`) n'est ajouté que si la mesure de la PR 10 l'exige. Résiduel toujours compris entre 0 et |montant|.
- Lettrage **total** (tous les résiduels du lot à 0) ou **partiel** : propriété calculée, non stockée.
- **Imputation** d'une saisie « au kilomètre » (N lignes, un appel) : débits et crédits sélectionnés sont imputés **dans l'ordre de l'échéance effective, à défaut de la date comptable, puis du rang de chaîne** (consensus Tryton `:1873-1874` et Odoo `:2735-2745` sur l'ordre ancien d'abord ; le rang de chaîne départage et rend le résultat déterministe), jusqu'à épuisement du côté le plus petit ; le reste demeure ouvert.
- **Devise** : la devise de la ligne est celle de tenue en L3. Une ligne en **devise étrangère** n'est lettrée que si les montants en devise imputés sont aussi égaux des deux côtés ; sinon 422 `lettrage_devise_hors_l3` (écarts de change : L6, spec §26 point 6). La forme exacte est précisée par la PR 9a.

### 2. Conditions d'un lettrage
Même compte ; compte **lettrable** (marque de données `lettrable` : comptes collectifs, 585, 588 ; sources p. PDF 430 ; un compte d'attente désigné par le dossier pour les relevés est déclaré lettrable à sa désignation : ADR 0034, point 8) ; **même tiers au sens de la fusion** (ADR 0031) ; lignes **validées** ; aucun contrôle de période ni de verrou (consensus A9).
- *Le tiers est un choix de Tryton* (`:676-682`) ; **Odoo ne le contrôle pas**. Les deux divergent : Sigma retient Tryton parce que le texte prévoit, « en principe », un sous-compte individuel par tiers dans lequel le lettrage se fait (ADR 0031, point 1), et signale le choix. Pour 585 et les comptes d'attente, aucune ligne n'a de tiers : la condition est satisfaite.
- *Lignes validées seulement* n'est **pas un contrôle plus strict** : une ligne de brouillon n'a pas d'identité stable (le brouillon est supprimé à la validation, ADR 0011) et un lien ne peut la citer ; Tryton et Odoo lettrent des lignes non validées, Sigma ne le peut pas.
- *Pas de contrôle de période* : comme les deux moteurs. Un lettrage ne change aucun solde ni aucune écriture : balance, grand livre et soldes de comptes sont identiques avant et après (propriété de test).

### 3. Datation : la date de l'**opération**, pas celle des pièces
`lettrage.date` et `lettrage_rupture.date` sont le **jour du dossier dans son fuseau** (ADR 0027) à l'instant de l'opération, passé en paramètre à `compta-core` (règle absolue 3) ; l'utilisateur ne la saisit pas. *Écart assumé avec **les deux moteurs et OCA**, qui datent par la plus haute date des lignes (voir plus haut) :* cette datation est indispensable pour reconstituer **à une date passée** une balance âgée stable (le résultat à une date ne dépend d'aucune opération postérieure, ADR 0033) et pour qu'une rupture soit datée (P3). Conséquence assumée : un règlement du 20 février lettré le 10 avril apparaît, dans une balance âgée au 28 février, comme un crédit **non lettré** à cette date, ce que montraient les livres à ce moment-là.

### 4. Lettre
Code **par (dossier, compte)** : suite `A … Z, AA … AZ, BA …` (numération bijective en base 26), **jamais réutilisée**, tirée d'un compteur `compteur_lettrage (compte, dernier)` pris `FOR UPDATE` (état dérivé `D`, créé à la demande comme `chaine_tete`, ADR 0020 point 2). Une ligne partielle peut porter **plusieurs lettres** (une par opération non rompue) ; l'affichage « groupe » est calculé à la lecture, jamais écrit (l'approche d'Odoo, qui réécrit le numéro sur les lignes à chaque changement, est écartée). Une rupture ne libère pas sa lettre. *Provenance : spec §10.1 pour le principe « par compte » ; aucun moteur n'a de lettres alphabétiques.*

### 5. Dé-lettrage : rupture en ajout seul, refus après clôture de l'exercice du lettrage (B2)
*Provenance : **choix Sigma**. Tryton et Odoo suppriment sans trace ni contrôle ; P3 l'interdit, donc la **rupture** est tranchée par le principe. Le refus après clôture vient de la **spec §10.1** (« possible tant que l'exercice n'est pas clôturé »), que le plan suit sans l'amender. Aucun moteur ne contrôle la période : la spec est ici plus stricte que les moteurs, par choix du mainteneur.*
- Un lettrage rompu **ne compte plus** dans les résiduels ; **re-lettrer crée un nouveau lettrage** (nouvelle lettre). Les lignes de rupture portent date, auteur, motif (texte de 400 octets au plus, sans caractère de contrôle ni de format) ; l'audit chaîné ne reprend que les identifiants.
- **Refus 409 `lettrage_exercice_clos`** si la date du **lettrage à rompre** tombe dans un exercice au statut clos. Raison : les à-nouveaux de N+1 sont « détaillés par tiers » et « les lettrages ouverts restent rapprochables » (spec §15.3) ; une rupture dans N après sa clôture désynchroniserait ceux de N+1 (L6). Une erreur de lettrage d'un exercice clos se corrige par un **nouveau lettrage** de l'exercice ouvert.
- Choix de l'option 2 (aucun contrôle, comme les moteurs) : **amendement de la spec §10.1**, que le mainteneur décide ; la PR 9a ne livre que l'option 1.
- Cas limite signalé : un lettrage dont la date tombe déjà dans un exercice clos (clôture anticipée) est **accepté** (aucun contrôle de période) mais ne pourra plus être rompu ; la PR 9a peut le refuser par un avertissement ou un refus, à instruire.
- Comme Tryton (`:605-634`), la rupture d'un lettrage dont une ligne est une écriture d'écart (point 6) rend un **avertissement**.

### 6. Écart de règlement (escompte, arrondi) : écriture au brouillard, jamais générée seule
Un lettrage ne crée **aucune écriture**. Si les montants diffèrent, l'utilisateur demande une **écriture d'écart au brouillard** (journal d'opérations diverses, comptes **désignés par le dossier**, aucun compte par défaut codé : règle absolue 9, ADR 0026) ; un humain la **valide** (la validation automatique n'est admise que pour les à-nouveaux et la reprise, spec §15.3) ; elle est ensuite lettrée avec les lignes. *Convergence : Tryton `WriteOff` (comptes configurés, `:1932-1955`) et Odoo/OCA (lignes d'écart par modèle, `account_reconcile_model.py:248-279`) ; **écart** : Tryton comptabilise l'écart automatiquement (`:1663`), Sigma non (la validation est un acte humain).* La forme de la désignation (modèle nommé, à la Tryton, ou rôle de compte) est fixée par la PR 9a. Écart de change : L6.

### 7. Lettrage automatique : règles déterministes, en proposition
Résultat en **proposition** ; application automatique seulement si le **paramètre du dossier** l'autorise (désactivé par défaut, spec §10.1 ; l'acteur est alors `systeme`, comme la file de tâches, ADR 0011). Ce n'est **pas** de l'IA : les règles sont déterministes (règle absolue 6 inchangée ; l'appariement par IA est de L4 et n'écrit que des propositions). Ordre de la spec §10.1 :
1. **Référence** : la référence de la ligne, non vide après normalisation (casse, espaces), est contenue dans celle de l'autre (OCA `:1040-1057`, qui accepte une référence vide : non repris) ; même compte, même tiers résolu, sens opposés ;
2. **Montant exact** et même tiers, avec **candidat unique** (OCA `COUNT(*) = 1`) ;
3. **Combinaison bornée** (somme de factures = règlement) : fonctionnalité **Sigma**, absente des deux moteurs (Tryton n'a que l'approximation gloutonne, `:1828-1871`) ; **bornes** nombre de lignes et budget d'itérations, **valeurs initiales** : 6 lignes par combinaison et 100 000 évaluations par groupe (compte, tiers), réglées par la mesure en PR 3a et 9b ; au-delà d'un seuil, calcul en **tâche de fond** (file de tâches de L2) ; ordre de recherche fixé (échéance, rang de chaîne) pour que le résultat ne dépende pas de l'ordre des lignes.
Aucune règle n'évalue de code (Tryton `simple_eval`, non repris : F11). Une proposition **ne mélange jamais** deux comptes, deux tiers résolus ni un groupe déséquilibré.

### 8. Concurrence et ordre des verrous
PostgreSQL ne permet pas de verrouiller une ligne sans `UPDATE` ; les lignes validées sont en ajout seul. Toute mutation de lettrage ou de révision d'échéance prend donc le **compteur du compte** (`compteur_lettrage`, ADR 0033 pour la révision). Ni la tête de chaîne ni le compteur d'écritures ne sont pris : lettrer ne gêne pas une validation. Ordre **unique et total** des verrous, qui **complète** l'ADR 0020 (point 1) sans le contredire :

| Rang | Verrou | Pris par |
|---|---|---|
| 1 | calendrier : exercices puis périodes, `FOR SHARE` (clôture : période `FOR UPDATE`) | validation, **rupture** (pour lire le statut de l'exercice), clôtures |
| 2 | brouillon `FOR UPDATE` | brouillon, validation |
| 3 | comptes puis tiers `FOR SHARE`, par ordre croissant ; compte de trésorerie `FOR SHARE` (ADR 0034) ; **fusion et archivage : tiers `FOR UPDATE`** à cette place | validation, lettrage, trésorerie, tiers |
| 4 | pièces `FOR SHARE` | validation |
| 5 | tête de chaîne `FOR UPDATE` | **validation seule** |
| 6 | compteur du journal et de l'exercice `FOR UPDATE` | **validation seule** |
| 7 | `compteur_lettrage` (dossier, compte) `FOR UPDATE` | lettrage, rupture, révision d'échéance |
| 8 | `compteur_pointage` (compte de trésorerie) `FOR UPDATE` | import, pointage, rupture de pointage, figement (ADR 0034) |
| 9 | tête d'audit du dossier | toute mutation |

Un cas d'usage qui prend les rangs 7 et 8 les prend dans cet ordre. Aucune transaction ne prend à la fois les rangs 5-6 et 7-8 : **pas de cycle**. Les résiduels et le tiers résolu sont **relus après les verrous** (comme le brouillon à la validation, ADR 0020). Un lettrage porte sur **un seul compte** (un seul compteur) ; deux lettrages concurrents de la même ligne se suivent et ne dépassent jamais son montant. Une **seconde barrière** (déclencheur `BEFORE INSERT` sur `lettrage_lien`, sans `SECURITY DEFINER`) refusant un résiduel négatif est à évaluer par la PR 9a, comme le déclencheur de `0352` pour les pièces.

### 9. Audit, intégrité, autorisation, migrations
- Chaque mutation (lettrage, rupture, écart demandé, règle) crée une entrée d'audit chaînée **sans valeur personnelle** (identifiants, lettre, nombre de lignes, origine).
- Aucune écriture validée ni aucun contenu haché ne change : `sigma-verifier` répond « intègre » sur l'export d'un dossier lettré ; la lettre est lue par L8 (colonnes `EcritureLet` et `DateLet` du FEC).
- Cedar : l'action `Lettrer` existe (« L3, aucune politique ») ; spec §20 : administrateur, valideur, collaborateur ; refus par défaut. Détail : PR 6.
- Tables `lettrage`, `lettrage_lien`, `lettrage_rupture` : ajout seul (catalogue, `GRANT` sans `UPDATE` ni `DELETE`) ; `compteur_lettrage` : `D` ; `regle_lettrage` : configuration. Migrations **`0520` à `0539`**.

### Amendement du 2026-11-27 (point 3, états à une date ; décision du mainteneur, PR 3c)
- **Un lettrage compte à une date D** si **toutes ses lignes** ont une date comptable au plus tard D et s'il **n'est pas rompu** dans le registre lu. C'est la règle convergente de Tryton et d'Odoo (date du lettrage = plus haute date des lignes, `move.py:555-557`, `account_partial_reconcile.py:64-67`) et d'OCA (`aged_partner_balance.py:99-141`) : principe « si Odoo et Tryton convergent, Sigma fait pareil ». Un règlement du 30 janvier lettré le 3 février est **soldé** dans une balance âgée au 31 janvier ; la date de l'opération y montrait un faux retard.
- **Les dates des opérations** (lettrage et rupture) **n'interviennent pas dans les états à une date** : elles servent à l'audit, à l'ordre du rejeu et au refus d'une rupture après clôture (point 5). Une rupture vaut à toute date, comme la suppression chez les moteurs : tenir pour actif à D un lettrage rompu après D le compterait en même temps que le lettrage qui l'a remplacé (une facture relettrée avec un autre règlement serait imputée deux fois). Même règle que l'état de rapprochement (ADR 0034, amendement du point 9).
- La « conséquence assumée » du point 3 et l'alternative écartée « date du lettrage = plus haute date des lignes » sont remplacées par ce qui précède. Une édition à une date passée change donc si un lettrage est fait ou rompu ensuite, comme chez les moteurs.

### Précisions de la PR 9a (2026-11-28)
- **Point 5, exercice « en clôture »** : seul le statut **clos** (`cloture`) refuse la rupture ; un exercice `en_cloture` ne la refuse pas (la spec §10.1 dit « tant que l'exercice n'est pas clôturé »).
- **Point 5, date de la rupture** : une rupture ne précède jamais son lettrage. Si le jour du dossier est antérieur à la date du lettrage (fuseau du dossier changé entre-temps, ADR 0027), la rupture prend la date du lettrage ; l'audit garde le jour du dossier (`jour_du_dossier`). Aucun refus : la spec et les moteurs n'en prévoient pas.
- **Point 1, devise** : le refus d'une ligne en devise étrangère hors du cas admis a pour code `lettrage_devise_etrangere` (422) ; le nom `lettrage_devise_hors_l3` n'est pas admis par la liste fermée des codes de l'API (minuscules et `_`, sans chiffre).
- **Point 8, seconde barrière** : retenue. Le déclencheur `lettrage_lien_controle` (migration `0520`) prend lui-même le compteur du compte `FOR UPDATE` et contrôle sens, compte, même tiers et résiduel ; il reste correct sans le verrou de l'API.

## Conséquences
- `compta-core` porte le lettrage pur (PR 3a) : invariants, résiduels, imputation, propositions ; propriétés `proptest` : un lettrage ne change aucun solde, débits imputés = crédits imputés, le dé-lettrage rétablit les résiduels, lettrer puis rompre puis relettrer redonne les mêmes résiduels, les propositions sont déterministes.
- Le tiers « résolu » est lu en un seul endroit (ADR 0031) ; la balance âgée et le lettrage s'accordent (ADR 0033).
- La spec §26 est à compléter (plan C11) : lettrage partiel et rupture (B1, B2), même tiers (A'10, C4). Le refus après clôture dépend de l'amendement éventuel de §10.1.
- **Risque** : contention sur un compte très actif (585, 411 générique) ; opérations courtes, un lettrage au kilomètre = une transaction, mesure en PR 9a.

## Alternatives écartées
- **Lettrage total seulement avec scission** (Tryton) : crée des écritures, contraire à la sobriété du ledger.
- **Résiduel écrit sur la ligne** (Odoo stocké, Tryton `reconciliation`) : mutation d'une écriture validée.
- **Suppression du lien au dé-lettrage** (Tryton, Odoo) : P3 ; le solde ouvert à une date passée deviendrait faux.
- **Numéro de lettrage dérivé du graphe** (Odoo) : réécrit sur les lignes ; **séquence par société** (Tryton) : la spec demande une lettre par compte.
- **Date du lettrage = plus haute date des lignes** (Tryton, Odoo, OCA) : rend une édition à date passée dépendante d'opérations postérieures (point 3).
- **Tolérance de lettrage** (Bigcapital, flottant à 0,01) : décimal exact.
- **Évaluation de code dans les règles** (Tryton `simple_eval`) et **combinaison non bornée** : déni de service (F2.16).
- **Lettrer des lignes de brouillon** : sans identité stable.
