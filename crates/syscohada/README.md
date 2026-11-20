# `syscohada` — liste officielle des comptes

`data/comptes.toml` contient les **1 401 comptes** du SYSCOHADA révisé :
- 85 à deux chiffres, 430 à trois chiffres, 886 à quatre chiffres ;
- les 9 classes.

Le crate les charge par `include_str!`, puis les valide à chaque chargement : `Referentiel::officiel()`.

## Source et méthode
- **Source qui fait foi** : AUDCIF et Système comptable OHADA, JO OHADA n° spécial du 15/02/2017, Titre VII, section 3, « Liste des comptes ». Ce sont les pages PDF 219 à 272 ; la page imprimée vaut la page PDF moins 3.
  - Le champ `page` de chaque compte renvoie à la page PDF.
  - Seuls les numéros et intitulés sont repris. Les notes de bas de page ne le sont pas.
- **Extraction** :
  - Point de départ : l'extrait OCR des références locales.
  - Accents restitués lorsque l'intitulé coïncidait exactement, à la casse et aux accents près, avec celui du module Tryton `account_syscohada`.
  - Chacune des 54 pages a ensuite été **relue sur l'image du PDF** et corrigée ligne par ligne : erreurs d'OCR, comptes manquants ou fusionnés, numéros mal lus. Par exemple, l'OCR lisait « 6383 Mécénat » au lieu de 6583, et « 15 Autres produits » au lieu de 75.
- **Casse** : celle du texte officiel. Les comptes à deux et trois chiffres sont en majuscules. Certains intitulés de sous-comptes sont elliptiques, tels qu'imprimés : « 1661 sur emprunts obligataires », sous « 166 INTÉRÊTS COURUS ».

## Écarts volontaires avec le texte imprimé
**Coquilles manifestes corrigées :**
- 130 « INSANCE » devient « INSTANCE » ;
- 27 « IMMOBLISATIONS » devient « IMMOBILISATIONS » ;
- 393 « APPOVISIONNEMENTS » devient « APPROVISIONNEMENTS » ;
- 656 « COMMERCIALE » devient « COMMERCIALES » ;
- accents manquants restitués sur des intitulés en majuscules.

**Numéros ou intitulés restitués**, signalés par le champ `note` (3 comptes) :

| Compte | Texte imprimé | Retenu | Raison |
|---|---|---|---|
| 4125 | « 412 Organismes Internationaux, Effets à recevoir » | 4125 | Numéro tronqué. Analogie avec 4115 ; Tryton retient aussi 4125. |
| 4794 | « Diminution des dettes d'exploitation » | « Diminution des dettes financières » | Doublon de 4793. Symétrie avec 4784. |
| 908 | « 907 AUTRES ENGAGEMENTS ACCORDÉS » | 908 | Doublon de 907. Sous-comptes 9081 à 9088 ; « Engagements accordés 905 à 908 ». |

**Non repris :**
- les plages « 911 à 914 » et « 915 à 918 » sous le compte 91 : le texte n'en donne pas les comptes ;
- les notes de bas de page.

## Champs qui ne viennent pas de la liste officielle
- **`journal_tresorerie`** et **`page_fonctionnement`** : compte admis comme compte d'un journal de trésorerie (spec §5.2), marque héritée par ses sous-comptes dans le plan du dossier. La page est celle de la fiche « Contenu et fonctionnement des comptes » qui fonde la marque.
  - **Admis** : 52 Banques (p. PDF 417), 53 Établissements financiers et assimilés (p. 419), 55 Instruments de monnaie électronique (p. 424-425), 57 Caisse (p. 428).
  - **Exclus** : 50 et 51 (p. 413, 415) ; 54, qui fonctionne contre le 52 ou le 53 (p. 420) ; 56, une dette dont la contrepartie est le 52 (p. 426) ; 585 et 588, comptes de passage des virements internes à solder, ligne de contrepartie et jamais compte d'un journal (p. 430-431) ; 59, dépréciations (p. 433).
  - **Exclusion explicite** (`journal_tresorerie = false`, l'emporte sur l'héritage) : 526 et 536, intérêts courus : ni encaisse ni avoir disponible. Les fiches des comptes 52 et 53 (p. PDF 416 et 418) ne font que les lister en subdivision.
  - **581, 582 : en attente du mainteneur** (régies d'avances et accréditifs, cas ambigu, p. 430) ; exclus d'ici là.
- **`tiers`** et **`page_tiers`** : types de tiers (spec §5.3) admis sur le compte et ses sous-comptes comme compte collectif d'un tiers ; la marque la plus proche s'applique. La page est celle de la fiche « Contenu et fonctionnement des comptes » qui fonde la marque.
  - **40** « Fournisseurs et comptes rattachés » (p. PDF 380, « tiers auxquels l'entité a recours pour ses achats ») : `fournisseur` ;
  - **41** « Clients et comptes rattachés » (p. PDF 384, « tiers auxquels l'entité vend les biens ou services ») : `client` ;
  - **42** « Personnel » (p. PDF 389, personnes liées à l'entité par un contrat de travail) : `salarie` ;
  - **461, 462, 463, 465, 467**, apporteurs et associés de la fiche 46 (p. PDF 399) : `associe`.
  - **Aucun tiers** en L2 sur les autres collectifs : 466 « Groupe, comptes courants », 103, 109, 18x, 45x, 48x. Le type `autre` n'est admis nulle part. Un tiers client et fournisseur est deux tiers, chacun sur son compte (fiches 40 et 41 : aucune compensation).
- **`nature`** : déduite de l'intitulé officiel de la classe.

  | Comptes | Nature |
  |---|---|
  | Classes 1 à 5 | bilan |
  | Classe 6 ; comptes 81, 83, 85, 87 et 89 | charge |
  | Classe 7 ; comptes 82, 84, 86 et 88 | produit |
  | Comptes 90 et 91 | hors bilan |
  | Comptes 92 à 99 | comptabilité analytique de gestion |

- **`sens`**, le sens normal du solde : la liste officielle ne le donne pas. Il est **déduit du texte** de l'AUDCIF : règle du chiffre 9 (Titre VII, ch. 2, § 1.2.1 « Constantes », p. PDF 216-217) et fiches « Contenu et fonctionnement des comptes » (Titre VII, ch. 3, p. PDF 274 à 539 : « est débité de… », « solde débiteur »…). Il sert à l'affichage et aux contrôles de vraisemblance, jamais au calcul. L'ensemble reste **à faire valider par un expert-comptable** (spec §26, point 14). Règles, du préfixe le plus long au plus court :
  - **Par défaut, selon la classe** : 1 et 7 créditeurs ; 2, 3, 5 et 6 débiteurs ; 4 et 9 variables. **Classe 8** : comptes à terminaison impaire (charges) débiteurs, à terminaison paire (produits) créditeurs (p. 217).
  - **Règle du chiffre 9** (p. 216) : un compte à trois chiffres ou plus terminé par 9 a le sens inverse de son parent (409, 419, 4449, `60x9` créditeurs, `70x9` débiteurs…), sauf en classe 2. Pour les comptes à deux chiffres, la terminaison 9 désigne les dépréciations et provisions (29, 39, 49, 59 créditeurs) ; dans les comptes de gestion, elle désigne les opérations sur provisions (659, 759…), sans inversion.
  - **Créditeurs** : 28 ; 56 ; 40 ; 42, sauf 421 ; 43 ; 441, 442, 443, 4441, 446 et 447 ; 462 et 465 ; 477 et 479 ; 481, 482 et 484 ; **181 à 183**, dettes liées à des participations (p. 309-310) ; **905 à 908**, engagements accordés (p. 506-507).
  - **Débiteurs** : 109, 129 et 139 ; 41 ; 421 ; 445 ; 467 ; 476 et 478 ; 485 et 488 ; **1962**, actif du régime de retraite (p. 313) ; **581 et 582**, régies d'avances et accréditifs (p. 430-431) ; **901 à 904**, engagements obtenus (p. 506-507).
  - **Variables** (« débiteur ou créditeur » selon le texte) : 12 et 13 (p. 275, 294, 296) ; **104**, compte de l'exploitant, « systématiquement soldé » (p. 283-284) ; variations de stocks 603 et 73 (p. 440, 477) ; comptes de liaison 184 à 188 (p. 310) ; virements internes 585 et 588.
  - **Charges à payer et produits à recevoir** (4286/4287, 4386/4387, 4486/4487, 5261/5267) : respectivement créditeurs et débiteurs. La fiche du compte 52 (p. 416) donne d'autres subdivisions que la liste officielle (5261 « Banques, en monnaie locale », 5265) : incohérence signalée à l'expert-comptable.
  - **Comptes de regroupement** : un compte prend le sens commun de ses subdivisions terminées par 1 à 8 (celles en 9 sont inverses par construction) ; il est variable si elles divergent. Ainsi 40 est créditeur malgré 409, et 601 débiteur malgré 6019.
  - **Soldes admis sans être normaux** (non modélisés à ce stade) : banques 52 et 53 créditrices (découvert, p. 416), 103 débiteur (p. 281).
- **`collectif`** : compte collectif de tiers (spec §5.1, invariant 4 du §6.2) ; les sous-comptes en héritent dans le plan du dossier.
  - **Repris des comptes `party_required` du module Tryton `account_syscohada`** : 103, 109, 181 à 185, 401, 402, 408, 409, 411, 412, 414 à 416, 418, 419, 421, 451, 452, 461 à 463, 465, 466, 481, 482, 485.
  - **Ajouts de Sigma Books**, comptes de tiers de la liste officielle absents de Tryton : 404, 413, 467 et 488 « Autres créances H.A.O. » (créances sur cessions d'immobilisations notamment ; décision du mainteneur du 2026-11-18, en accord avec Odoo `l10n_syscohada`, qui rend 488 lettrable).
  - Voir `docs/provenance/tryton.md`.

## Compte des journaux par défaut (`data/journaux.toml`)
Les journaux par défaut d'un dossier (spec §5.2) sont décrits sans compte dans `crates/compta-core/data/journaux-par-defaut.txt`. Le compte de chaque journal de **trésorerie** par défaut est une donnée de ce crate, avec sa source :

| Journal | Compte | Source | Motif |
|---|---|---|---|
| CA (caisse) | 5711 « Caisse en monnaie nationale » | Liste des comptes, p. PDF 252 | 571 se subdivise en 5711 et 5712 ; on ne saisit que sur un compte sans subdivision. |

Le chargement refuse un compte absent de la liste officielle ou subdivisé. Les journaux BQx et MMx ne sont pas des journaux par défaut : leur compte est fourni à leur création. `Referentiel::journaux_par_defaut(plan)` instancie les journaux d'un nouveau dossier.

## Comparaison avec Tryton et Odoo
| | Tryton `account_syscohada` (2016, `f5354cc`) | Odoo 19.0 `l10n_syscohada` (`fa175fb`) |
|---|---|---|
| Comptes hors liste officielle | 21 : 1666, 1676, 2715, 2766, 2821, 2825, 4126, 4442, 4714, 4727, 4728, 4981 à 4983, 5361, 5367, 611, 6235, 6721, 687, 872 | 39 : 2715, 2766, 4113, 4445, 469, 4856, 6388, et des sous-comptes inventés sous 91 (9111 à 9188) |
| Comptes officiels absents | 149, dont toute la classe 9 et des comptes ajoutés en 2017 (2184, 4016, 404x, 413x, 551 à 558…) | 9 comptes à quatre chiffres (5181 à 5187, 5211, 5215, 5261, 5267) |
| Intitulés | Souvent paraphrasés ; décalages sous 168 (1684 à 1686) et 571 à 573 | Paraphrasés (ex. 1011 « Capital engagé non appelé ») |

Aucune des deux références n'est fiable seule : **le PDF officiel a tranché chaque écart**.
