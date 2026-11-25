# 0026 — Comptes de TVA désignés ; pas de contrôle bloquant étiquettes et comptes à la validation

- **Statut :** proposée
- **Date :** 2026-11-25

## Contexte
L'ADR 0019 (§9) a posé `controler_taxes_et_comptes` (le total des étiquettes « taxe » égale le solde des comptes de TVA) et l'ADR 0020 (§7) l'a laissé hors de la validation, faute d'une donnée de `syscohada` qui désigne les comptes de TVA (règle absolue 9). L'issue #189 demandait cette donnée et un refus à la validation en cas d'incohérence entre une étiquette et son compte. Une première version de cette PR l'implémentait (rapprochement ligne à ligne, centralisation en exception, deux codes 409). Le mainteneur a tranché le 2026-11-25 : **on fait ce que Odoo et Tryton font** (principe général : si Odoo et Tryton convergent sur un mécanisme, Sigma fait pareil).

## Questions comptables instruites (textes officiels)
1. **Quels comptes portent la TVA ?** Liste officielle des comptes (AUDCIF + SYSCOHADA, JO OHADA 15/02/2017, p. PDF 243-244) : 443 « ÉTAT, T.V.A. FACTURÉE » (4431 sur ventes, 4432 sur prestations de services, 4433 sur travaux, 4434 sur production livrée à soi-même, 4435 sur factures à établir), 444 « ÉTAT, T.V.A. DUE OU CRÉDIT DE T.V.A. » (4441 due, 4449 crédit à reporter), 445 « ÉTAT, T.V.A. RÉCUPÉRABLE » (4451 à 4456, dont 4455 sur factures non parvenues). 446 est « autres taxes sur le chiffre d'affaires », hors TVA. Les pages 243-244 ont été lues sur le PDF scanné (sans texte exploitable) ; l'OCR de `references/extraits/` a deux coquilles (4434, 4435), déjà corrigées dans `comptes.toml`.
2. **Quel rôle pour chaque compte ?** La fiche de fonctionnement du compte 44 (p. PDF 394-395) est générale (dettes d'impôts créditées par le débit des charges, règlements débités par le crédit de la trésorerie) et ne distingue pas la TVA. Le Guide d'application (1er mai 2017) le dit : la TVA facturée est enregistrée « dès l'émission de la facture » au 443, dette envers l'État (§4.4 p. PDF 31, §2.2 p. PDF 35) ; la TVA déductible des achats est une créance sur l'État, au débit du 445 (id.) ; en fin de période, 443 et 445 sont virés au 444, créditeur (4441) ou débiteur (4449) (§2.2 p. PDF 35, application 7 p. PDF 36).
3. **La « TVA probable » (4435, 4455).** À la clôture, « il est tenu compte de la T.V.A. probable » : au débit du 4455 en général et du 4435 dans le dernier cas (p. PDF 89), au crédit du 4455 pour l'avoir attendu du fournisseur et du 4435 pour la facture à adresser au client (p. PDF 94). Ce sont des provisions de fin de période, ni exigibles ni déductibles tant que la facture n'existe pas : rôle `provision`, distinct du 443 et du 445 qui les contiennent.
4. **Le sens débit ou crédit d'une ligne de TVA est-il fixé ?** Non : le 4455 et le 4435 sont débités et crédités selon le cas (p. PDF 89 et 94), un avoir débite le 443.

## Ce que font les moteurs de référence (lecture réelle, 2026-11-25)
- **Odoo** (`addons/account` : `account_tax.py:578-612`, `account_move_line.py:213-228`, lignes de répartition) : la répartition (base ou taxe, compte, étiquettes) est contrôlée dans sa structure, jamais dans la cohérence entre la nature ou le sens d'une ligne de taxe et son compte, ni par une égalité entre les étiquettes et les soldes des comptes de TVA. Avoir : répartition de remboursement symétrique (`:578-612`) ; autoliquidation +100/−100 (`:215, :1209, :1246-1256`) : le sens débit ou crédit n'est jamais imposé. L'exigibilité est portée par `tax_exigibility` et `cash_basis_transition_account_id` (`:166-176, 269-277`) et l'exclusion du rapport par les étiquettes (`tax_tag_ids`), pas par une exemption de compte. `l10n_syscohada` ne désigne aucun compte de TVA.
- **Tryton** (`modules/account/tax.py:1339-1430`, `move.py:1414`) : les lignes de taxe (`account.tax.line`) sont créées par la facture ; aucun contrôle de cohérence avec le compte ni d'égalité avec les soldes. L'exigibilité à l'encaissement est dans `account_tax_cash` (`account.py:55-150`). `account_syscohada` ne désigne aucune taxe.
- **Aucun** des deux ne traite la « TVA probable » (4435, 4455) : cette désignation vient du seul Guide p. PDF 94. Aucun ne traite la centralisation ni le paiement de la TVA (le code de clôture de TVA d'Odoo est dans `account_reports`, absent du clone ; OCA `account-closing` ne couvre que la clôture d'exercice ; Dolibarr ne fait que des comptes par défaut) : ce que le Guide p. PDF 35-36 en dit reste à instruire en L5.
- Aucun code ni test d'Odoo ou de Tryton n'est repris (pas de provenance). Sources d'inspiration pour les tests de L5 : Odoo `test_account_move_out_refund.py:989, :1112` et `test_account_move_out_invoice.py:3490-3500`.

## Décision
1. **La donnée est livrée** : `crates/syscohada/data/comptes-tva.toml` désigne 443 (`facturee`), 444 (`due`), 445 (`recuperable`), 4435 et 4455 (`provision`) avec intitulé, page de la liste, sens normal, page de la fiche de fonctionnement et source (documents et pages). Chaque chargement la valide contre `comptes.toml`. Un compte désigné vaut pour ses subdivisions ; la désignation la plus précise l'emporte. Elle sert la déclaration de TVA (L5) et la répartition par étiquette.
2. **Aucun contrôle bloquant de cohérence de TVA à la validation** (unitaire, par lot, contre-passation), comme Odoo et Tryton : ni `controler_taxes_et_comptes`, ni rapprochement ligne à ligne, ni code 409 dédié. L'étiquette fiscale reste facultative ; l'écriture validée n'est contrôlée que par ce qu'prévoient déjà les ADR 0019 et 0020.
3. **Le rapprochement étiquettes et soldes des comptes de TVA est un contrôle de préparation de la déclaration** (rapport d'écarts, non bloquant), à instruire en L5 : issue de suivi « [L5] Rapport d'écarts étiquettes fiscales / comptes de TVA à la préparation de la déclaration (suite de #189) ». `controler_taxes_et_comptes` (`compta-core`) y sera employé avec la désignation de `syscohada`.

## Risques assumés
- **Une étiquette incohérente avec son compte peut se figer dans la chaîne** (écriture validée définitive, étiquette dans le contenu haché, ADR 0019). Elle se corrige par contre-passation et nouvelle écriture.
- L'écart sera **visible dans le rapport de L5** avant la déclaration ; il n'est pas empêché à la saisie.
- Une écriture de TVA non étiquetée est acceptée : la déclaration, qui somme les étiquettes, l'ignorerait, d'où l'intérêt du rapport.

## Conséquences
- `syscohada` : `Referentiel::comptes_de_tva`, `ComptesTva::role`, `RoleCompteTva`. Aucune autre crate ne change ; la validation (ADR 0020) est inchangée.
- Les points laissés au Guide (centralisation vers 444, paiement de la TVA depuis le 444, provisions, avoirs et autoliquidation) sont à valider par l'expert-comptable lors de l'instruction de L5.

## Alternatives écartées
- **Refuser à la validation les lignes de taxe incohérentes et l'écart avec les soldes** (première version de cette PR) : plus strict qu'Odoo et Tryton, qui n'ont pas ce contrôle ; écarté par le mainteneur (2026-11-25).
- **Désigner les comptes par un préfixe dans `compta-core`** : viole la règle absolue 9.
- **Étiqueter la centralisation** : annulerait la taxe collectée dans la déclaration (somme par étiquette).
