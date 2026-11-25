# 0026 — Comptes de TVA désignés et rapprochement des étiquettes fiscales à la validation

- **Statut :** proposée
- **Date :** 2026-11-25

## Contexte
L'ADR 0019 (§9) a posé `controler_taxes_et_comptes` (le total des étiquettes « taxe » égale le solde des comptes de TVA) et l'ADR 0020 (§7) l'a laissé hors de la validation, faute d'une donnée de `syscohada` qui désigne les comptes de TVA (règle absolue 9). Sans ce contrôle, une étiquette incohérente avec son compte se fige dans la chaîne (écriture définitive) et la déclaration de TVA de L5, qui somme par étiquette, en hérite. Issue #189.

## Questions comptables instruites (textes officiels)
1. **Quels comptes portent la TVA ?** Liste officielle des comptes (AUDCIF + SYSCOHADA, JO OHADA 15/02/2017, p. PDF 243-244) : 443 « ÉTAT, T.V.A. FACTURÉE » (4431 sur ventes, 4432 sur prestations de services, 4433 sur travaux, 4434 sur production livrée à soi-même, 4435 sur factures à établir), 444 « ÉTAT, T.V.A. DUE OU CRÉDIT DE T.V.A. » (4441 due, 4449 crédit à reporter), 445 « ÉTAT, T.V.A. RÉCUPÉRABLE » (4451 sur immobilisations, 4452 sur achats, 4453 sur transport, 4454 sur services extérieurs et autres charges, 4455 sur factures non parvenues, 4456 transférée par d'autres entités). 446 est « autres taxes sur le chiffre d'affaires », hors TVA. Vérifié sur le PDF (le texte OCR a deux coquilles à 4434 et 4435, déjà corrigées dans `comptes.toml`).
2. **Quel rôle pour chaque compte ?** La fiche de fonctionnement du compte 44 (p. PDF 394-395) est générale (dettes d'impôts créditées par le débit des charges, règlements débités par le crédit de la trésorerie) et ne distingue pas la TVA. Le Guide d'application (1er mai 2017) le dit : la TVA facturée est enregistrée « dès l'émission de la facture » au 443, dette envers l'État (§4.4 p. PDF 31, §2.2 p. PDF 35) ; la TVA déductible des achats est une créance sur l'État, au débit du 445 (id.) ; en fin de période, 443 et 445 sont virés au 444, créditeur (4441) ou débiteur (4449) (§2.2 p. PDF 35, application 7 p. PDF 36).
3. **Le sens débit ou crédit d'une ligne de TVA est-il fixé ?** Non, et les textes montrent les deux : le 4455 est crédité pour l'avoir attendu du fournisseur, le 4435 pour la facture à établir (p. PDF 94) ; un avoir ou une régularisation débite le 443. Le rapprochement ne contraint donc que le **compte** (la famille), jamais le sens débit ou crédit.
4. **L'écriture de centralisation (443 et 445 virés au 444) a-t-elle une étiquette ?** Non, par construction : elle ne crée ni ne supprime de taxe, elle la déplace ; la déclaration somme les étiquettes des opérations. Étiqueter ses lignes annulerait la taxe collectée dans la déclaration.

## Décision
1. **Donnée** : `crates/syscohada/data/comptes-tva.toml` désigne 443 (rôle `facturee`), 444 (`due`) et 445 (`recuperable`) : numéro, intitulé, page de la liste, sens normal, page de la fiche de fonctionnement, source (documents et pages). Chaque chargement la valide contre `comptes.toml` (numéro officiel, intitulé, page et sens identiques ; trois rôles, une fois chacun ; aucun compte imbriqué ; page de fiche dans la plage ; source non vide). Un compte désigné vaut pour ses subdivisions (officielles, libres `4431-01`, par période d'imposition du Guide).
2. **Règle pure** dans `compta-core` : `rapprocher_etiquettes_et_comptes(lignes, role_du_compte)`, `RoleCompteTva`, `ErreurRapprochementTva`. Elle ne connaît aucun numéro ; la désignation lui est passée (règle absolue 3 et 9) :
   - une ligne « taxe » collectée porte un compte de TVA facturée, une ligne « taxe » déductible un compte de TVA récupérable, jamais un compte de charge, de produit, de tiers ni le 444 ;
   - une ligne « base » ne porte aucun compte de TVA ;
   - le total des étiquettes « taxe » égale le solde des comptes de TVA de chaque sens (`controler_taxes_et_comptes`) : une ligne sur un 443x ou 445x sans étiquette « taxe » est donc refusée ;
   - **exception unique** : l'écriture sans aucune étiquette dont toutes les lignes sont sur des comptes de TVA (443, 444, 445), la centralisation de la période (question 4).
3. **Application** : `preparer` (première étape pure de la validation, commune à la validation unitaire et au lot) lit la désignation (`comptes_tva()`, chargée au démarrage : le serveur refuse de démarrer si elle est invalide) et refuse en **409** avec deux codes stables : `etiquette_tva_incoherente` (ligne « taxe » sur un compte étranger à son sens, base sur un compte de TVA) et `ecart_tva_etiquettes` (total des étiquettes différent du solde des comptes de TVA). Par lot, le refus d'une écriture est rendu dans `refus` (`lot_refuse`), comme les autres : tout ou rien (ADR 0021).
4. **Pas de contrôle à la contre-passation** : le miroir copie les étiquettes, sens D/C inversés (ADR 0019 §1) ; il est cohérent si l'original l'est, et la contre-passation est la voie de correction d'une écriture validée avant ce contrôle, qui ne doit jamais être bloquée (règle absolue 5).
5. **Choix prudents** (refus plutôt qu'acceptation), à faire valider par l'expert-comptable :
   - une écriture qui touche le 443x ou le 445x sans étiquette « taxe », hors centralisation, est **refusée** (par exemple une écriture manuelle de TVA non étiquetée, ou un reclassement 4435 vers 4431 : la déclaration de L5 l'ignorerait) ; l'étiquette reste facultative pour les écritures qui ne touchent aucun compte de TVA ;
   - le 4449 (crédit de TVA à reporter) et le 4441 sont traités comme le 444 : aucune étiquette, centralisation seulement ;
   - le 446 (autres taxes sur le chiffre d'affaires) n'est pas un compte de TVA : une étiquette « taxe » dessus est refusée ;
   - l'exception de centralisation est la seule voie d'écrire un compte de TVA sans étiquette ; elle accepte aussi un virement direct 443 vers 445, que les textes ne prévoient pas explicitement (sans effet sur la déclaration).

## Comparaison avec les moteurs de référence (lus le 2026-11-25)
- **Tryton** (`modules/account/tax.py` `TaxLine`, `move.py` `Line.check_account`, `Move.validate_move`) : une `TaxLine` (type `base` ou `tax`, montant, taxe, ligne d'écriture) est créée par la facture, mais aucune contrainte ne rapproche la nature ou le sens de la ligne de taxe du compte de la ligne ; les comptes `invoice_account` et `credit_note_account` de la taxe ne servent qu'à générer les lignes. Aucun contrôle « étiquettes égales soldes » ; une `TaxLine` reste modifiable sur une pièce postée tant que la période est ouverte. `account_syscohada` ne contient que le plan (aucune taxe, aucune désignation de TVA).
- **Odoo** (`addons/account` `account.tax`, `account.tax.repartition.line`, `account.move.line`) : la répartition (base ou taxe, compte, étiquettes) est contrôlée dans sa structure (une ligne de base, facteurs de 100 %), pas dans la nature ou le sens de son compte ; aucun contrôle de la somme des lignes de taxe contre le solde des comptes de taxe ; les étiquettes de taxe (`tax_tag_ids`) sont hors du hachage et modifiables. `l10n_syscohada` ne désigne aucun compte de TVA pour les répartitions (seulement le plan).
- **Retenu** : rien n'est repris en code. Ni l'un ni l'autre ne fait le rapprochement ligne à ligne ni l'égalité des totaux : ils laissent le compte et l'étiquette diverger, ce que l'issue #189 veut interdire. **Écart assumé** : le rapprochement est un refus à la validation (étiquette figée dans la chaîne, ADR 0019), non un simple rapport.

## Conséquences
- `syscohada` : `Referentiel::comptes_de_tva`, `ComptesTva::role`. `compta-core` : `rapprocher_etiquettes_et_comptes`. `api` : deux codes 409 de `MotifConflit` ; `comptes_tva()` au démarrage.
- Les écritures déjà validées ne sont pas revues (définitives) ; leur contre-passation reste possible.
- Un dossier qui saisit la TVA doit désormais étiqueter ses lignes de TVA (base et taxe) pour valider.
- La désignation sert aussi à L5 (déclaration) : même donnée, un seul endroit.

## Alternatives écartées
- **Désigner les comptes par un préfixe dans `compta-core`** : viole la règle absolue 9.
- **Contrôler les soldes du dossier entier plutôt que l'écriture** : n'empêche pas de figer une étiquette incohérente dans la chaîne ; relève de L5 en complément.
- **Étiqueter la centralisation** : annulerait la taxe collectée dans la déclaration (somme par étiquette).
- **Contraindre le sens débit ou crédit** : avoirs, régularisations et factures non parvenues l'exigent dans les deux sens (question 3).
