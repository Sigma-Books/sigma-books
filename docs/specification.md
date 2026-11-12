# Sigma Books — Spécification fonctionnelle v1.0
## Logiciel comptable SYSCOHADA complet, piloté par l'IA

> **Statut :** spécification de référence pour le développement — novembre 2026
> **Choix techniques :** voir *Sigma Books — Fondations techniques* (Rust, PostgreSQL, SvelteKit, Cedar, AGPL-3.0, SaaS d'abord). Ce document décrit le **produit** ; les Fondations décrivent **comment** il est construit et sécurisé.
> **Décision de cadrage :** la v1 est le logiciel comptable complet — saisie, IA, trésorerie, tiers, TVA, fin d'exercice, états financiers.

---

## Sommaire
0. Cadrage
1. Produit
2. Principes non négociables
3. Périmètre v1
4. Organisation : cabinets, dossiers, exercices, périodes
5. Référentiel comptable
6. Cycle de vie des écritures et intégrité
7. Saisie et import
8. Pipeline IA
9. Trésorerie, relevés et déduplication
10. Tiers, lettrage et échéancier
11. TVA
12. Paie et notes de frais
13. Immobilisations
14. Travaux de fin d'exercice
15. Clôture et réouverture
16. Répartition du résultat
17. États financiers et éditions
18. Migration, interopérabilité et réversibilité
19. Portail client
20. Rôles et droits
21. Exigences de sécurité produit
22. Évaluations et tests de conformité
23. Modèle de données
24. Plan de développement
25. Après la v1
26. Points à faire valider par un expert-comptable

---

## 0. Cadrage

| Sujet | Décision |
|---|---|
| Nature du produit | Logiciel de tenue comptable complet, multi-dossiers, avec lecture IA des pièces |
| Client payant | Cabinets d'expertise comptable et de tenue ; entreprises dotées d'un comptable interne |
| Référentiel | SYSCOHADA révisé (AUDCIF 2017), **Système Normal**. Le Système Minimal de Trésorerie n'est **pas proposé** en v1 (aucun champ ni option tant qu'il n'est pas implémenté). |
| Pays v1 | Sénégal. Le modèle est multi-pays dès la conception (paramètres fiscaux par pays, versionnés). |
| Devise | XOF ; opérations en devises étrangères gérées (§6.6). |
| Langue | Français. |
| Licence | AGPL-3.0-or-later ; marque « Sigma Books » réservée. |

### Choix de conception structurants
| Sujet | Choix | Raison |
|---|---|---|
| Caractère définitif des écritures | **Brouillard modifiable → validation définitive** ; correction par contre-passation | Exigence de caractère définitif des enregistrements ; cohérence avec la chaîne de hachage (§6, §26) |
| Rôle de l'IA | L'IA alimente uniquement le **brouillard** ; un humain valide | Une confiance déclarée par un modèle de langage n'est pas calibrée |
| Retenues à la source | Comptes **447x** | Plan SYSCOHADA |
| TVA récupérable | **4451 / 4452 / 4453 / 4454** selon la nature | Plan SYSCOHADA |
| Cession d'immobilisation | Prix de cession **toujours** en 82x, VNC en 81x | Résultat de cession = 82x − 81x |
| Répartition du résultat | Affectations sur comptes de capitaux propres et de tiers ; perte en **129** par défaut | L'imputation d'une perte est une décision d'AG |
| Charges constatées d'avance | Charge à 100 % à la saisie, régularisation en 476 **à l'inventaire** | Pratique comptable |
| Déduplication | Module central (§9) | Éviter toute double comptabilisation |
| Coût IA | Quotas de pièces + coût mesuré | Marge |

---

## 1. Produit

### 1.1 Promesse
> Les pièces arrivent (photo, PDF, relevé, e-mail). Sigma Books les lit, prépare les écritures SYSCOHADA, rapproche la banque, et produit les états financiers. Le comptable contrôle et valide.

### 1.2 Utilisateurs
| Utilisateur | Besoin principal |
|---|---|
| Collaborateur comptable | Traiter vite et sans erreur les pièces de nombreux dossiers |
| Comptable valideur / expert-comptable | Contrôler, valider, clôturer, signer les états |
| Administrateur du cabinet | Gérer les dossiers, les équipes, les droits, la facturation |
| Dirigeant client (portail) | Envoyer ses pièces, répondre aux questions du cabinet, consulter quelques indicateurs |
| Auditeur / commissaire aux comptes | Consulter en lecture seule, vérifier l'intégrité |

### 1.3 Indicateurs produit (mesurés dans l'application)
- Temps moyen de traitement d'une pièce.
- Taux de propositions IA validées sans modification, par type de document.
- Part des mouvements de trésorerie justifiés par une pièce.
- Délai entre la fin de l'exercice et la production des états financiers.

---

## 2. Principes non négociables

**Comptables (AUDCIF)**
1. Partie double : ∑Débit = ∑Crédit pour chaque écriture, vérifié par le moteur (type `EcritureEquilibree`, cf. Fondations §2.4).
2. Pièce justificative : toute écriture validée référence au moins une pièce, ou porte le motif explicite « sans pièce » (écritures d'inventaire générées : la pièce est alors l'état de calcul produit par Sigma Books).
3. Chronologie et numérotation : numérotation continue, sans trou, par journal et par exercice, attribuée **à la validation**.
4. Caractère définitif : une écriture validée n'est ni modifiable ni supprimable. Correction = contre-passation + nouvelle écriture.
5. Périodicité : exercice, périodes mensuelles, verrouillage de période, clôture d'exercice irréversible.
6. Conservation : pièces et livres conservés 10 ans, exportables à tout moment.

**IA**
7. L'IA propose, l'humain valide. L'IA écrit uniquement dans le brouillard, jamais dans le journal validé.
8. Les montants viennent de l'extraction et sont recalculés par le moteur ; l'IA choisit les comptes, jamais les montants.
9. L'IA ne choisit que des comptes existant dans le plan du dossier.
10. Un document est une donnée, jamais une instruction.

**Sécurité** (détail : Fondations §3)
11. Refus par défaut ; triple isolation entre tenants ; documents traités en bac à sable ; intégrité prouvable par chaîne de hachage.

---

## 3. Périmètre v1

| Module | Contenu v1 |
|---|---|
| Organisation | Cabinets, dossiers, exercices, périodes, utilisateurs, rôles, portail client |
| Référentiel | Plan SYSCOHADA, sous-comptes, comptes auxiliaires, journaux, tiers, paramètres fiscaux Sénégal |
| Saisie | Saisie manuelle rapide, modèles d'écritures, abonnements, import CSV/FEC |
| IA | Factures achat/vente, reçus, relevés bancaires et mobile money, avis de virement, notes de frais, récapitulatifs de paie |
| Trésorerie | Comptes bancaires, mobile money, caisse ; import de relevés ; rapprochement bancaire ; déduplication ; mouvements à justifier |
| Tiers | Lettrage (auto et manuel, partiel), échéancier, balance âgée, relevé de compte tiers |
| TVA | Calcul par période, écriture de liquidation, état récapitulatif, verrouillage de période déclarée |
| Paie | Comptabilisation à partir d'un récapitulatif de paie (pas de calcul de paie) |
| Immobilisations | Registre, linéaire et dégressif, pro rata, cessions, mises au rebut, dotations |
| Fin d'exercice | Stocks (inventaire intermittent), charges à payer, produits à recevoir, CCA, PCA, provisions, impôt sur le résultat |
| Clôture | Liste de contrôle bloquante, clôture irréversible, report à nouveau automatique |
| Résultat | Assistant de répartition après AGO |
| États | Journaux, grand livre, balances, Bilan, Compte de résultat, TFT, notes annexes, PDF et Excel |
| Interopérabilité | Reprise de données (plan, tiers, balance d'ouverture, FEC), export FEC et profils d'export |

### Hors v1
Système Minimal de Trésorerie · comptabilité analytique · calcul de paie · inventaire permanent · consolidation · télédéclaration fiscale · paiements · relances clients automatiques · autres pays que le Sénégal · applications mobiles natives (le portail est une PWA).

---

## 4. Organisation : cabinets, dossiers, exercices, périodes

### 4.1 Hiérarchie
```
Cabinet (tenant)
└── Dossier (une entité juridique)
    ├── Référentiel (plan, journaux, tiers, paramètres)
    └── Exercice (N-1, N, N+1…)
        └── Période (mois)
```
- Une entreprise qui tient sa propre comptabilité est un cabinet d'un seul dossier.
- Un utilisateur peut appartenir à plusieurs cabinets (cas d'un expert intervenant pour plusieurs structures) ; chaque appartenance a son rôle.

### 4.2 Exercice
| Statut | Signification |
|---|---|
| `ouvert` | Saisie et validation possibles |
| `en_cloture` | Travaux de fin d'exercice ; saisie limitée aux journaux d'OD et d'inventaire |
| `cloture` | Irréversible. Aucune écriture possible. |

- Dates libres (exercice de 12 mois par défaut ; premier exercice plus court ou plus long autorisé dans les limites légales — paramètre à valider).
- Deux exercices peuvent être ouverts simultanément (N clôture en cours, N+1 courant).

### 4.3 Période
| Statut | Signification |
|---|---|
| `ouverte` | Saisie autorisée |
| `verrouillee` | Plus aucune validation dans la période (ex. après déclaration de TVA). Déverrouillage par un rôle habilité, motif obligatoire, journalisé. |
| `cloturee` | Suit la clôture de l'exercice ; irréversible |

---

## 5. Référentiel comptable

### 5.1 Plan de comptes
> Tous les numéros de comptes cités dans cette spécification ont été vérifiés contre la **liste officielle des comptes** du SYSCOHADA révisé (AUDCIF + SYSCOHADA, JO OHADA n° spécial du 15/02/2017, Titre VII, section 3).

- Plan SYSCOHADA révisé chargé à la création du dossier (comptes de 2 à 4 chiffres, intitulés officiels, classe, sens normal, nature).
- **Sous-comptes libres** sous un compte officiel (`6052-01`, `521-BICIS`). Le **tronc** (compte officiel parent) est obligatoire et immuable.
- Les états financiers agrègent **toujours par tronc** (§17).
- Comptes **auxiliaires** (tiers) rattachés à un compte collectif (401, 411, 421…).
- Un compte ne peut être supprimé s'il a des écritures ; il peut être désactivé.

### 5.2 Journaux
Journaux par défaut (codes modifiables) :

| Code | Journal | Type |
|---|---|---|
| AC | Achats | achats |
| VT | Ventes | ventes |
| BQx | Un par compte bancaire | trésorerie |
| MMx | Un par portefeuille mobile money | trésorerie |
| CA | Caisse | trésorerie |
| PA | Paie | opérations diverses |
| OD | Opérations diverses | opérations diverses |
| IN | Inventaire (fin d'exercice) | opérations diverses |
| AN | À-nouveaux | à-nouveaux (généré uniquement) |

### 5.3 Tiers
Nom, NINEA, RCCM, adresse, contacts, type (client, fournisseur, salarié, associé, autre), compte collectif, compte auxiliaire, conditions de paiement, devise, statut fiscal (assujetti TVA, soumis à retenue).

### 5.4 Paramètres fiscaux (par pays, versionnés)
- TVA : taux normal 18 % ; taux réduits et opérations exonérées **à sourcer dans le CGI** avant mise en production.
- Comptes TVA : 4431 (collectée), 4451/4452/4453/4454 (récupérable selon la nature), 4441 (TVA due), 4449 (crédit à reporter).
- Retenues à la source : types, taux et comptes 447x — **à sourcer dans le CGI**.
- Référence fiscale Sénégal : Code général des impôts (loi n° 2012-31) **consolidé par la DGID à janvier 2025** (intègre la LFI 2025), **modifié par la loi n° 2025-17 du 27 septembre 2025** (JO n° 7853 du 02/10/2025). Les lois de finances ultérieures sont intégrées à chaque nouvelle version des paramètres.
- Mesures de la loi n° 2025-17 qui touchent directement la tenue des dossiers :
  - **prélèvement de 0,5 %** opéré par l'opérateur de transfert d'argent sur les paiements reçus par les titulaires d'un code marchand (art. 220 ter) — il réduit les encaissements mobile money et doit être comptabilisé ;
  - **taxe sur les opérations de transfert d'argent** de 0,5 %, plafonnée à 2 000 F par transaction (art. 395 et suivants) — elle apparaît dans les frais des relevés mobile money.
- Seuil d'immobilisation : paramètre du dossier.
- Coefficients du dégressif : **paramètre fiscal à sourcer** (§13.3).
- Chaque écriture enregistre la **version des paramètres** utilisée.

### 5.5 Devises
- Devise de tenue : XOF.
- Une ligne en devise conserve : devise, montant en devise, taux, montant en XOF.
- Table des cours (saisie manuelle ou import). Écarts de change à l'encaissement/décaissement et écarts de conversion à la clôture (§14.7).

---

## 6. Cycle de vie des écritures et intégrité

### 6.1 Statuts
```
                ┌──────────── BROUILLARD (modifiable, supprimable) ────────────┐
 proposee_ia ─► a_revoir ─► prete ─┐                                            │
 saisie_manuelle ──────────────────┤                                            │
 import / modele / abonnement ─────┘                                            │
                └───────────────────────────────────────────────────────────────┘
                                   │ VALIDATION (rôle habilité, ré-authentification
                                   │ pour la validation par lot)
                                   ▼
                              validee  ──► (contre-passée : reste validée, liée à
                         (définitive)       son écriture d'annulation)
```
- **Brouillard** : l'écriture n'a pas de numéro définitif, n'apparaît dans les états qu'en mode « provisoire » (option d'affichage), et peut être modifiée ou supprimée. Chaque modification est journalisée (avant/après).
- **Validation** : contrôle des invariants (§6.2), attribution du numéro définitif, calcul du hash, inscription au journal. Irréversible.
- **Contre-passation** : écriture miroir, validée, liée à l'originale. Aucune écriture validée n'est jamais modifiée.

### 6.2 Invariants vérifiés à la validation
1. ∑D = ∑C (en XOF ; et par devise pour les lignes en devise).
2. Chaque ligne a un débit **ou** un crédit, strictement positif.
3. Tous les comptes existent, sont actifs et sont des comptes de **mouvement** (pas un compte de regroupement).
4. Une ligne sur un compte collectif porte un tiers.
5. La date est dans une période `ouverte` d'un exercice `ouvert` (ou `en_cloture` pour les journaux OD/IN).
6. Au moins une pièce rattachée, ou un motif « sans pièce » autorisé pour ce journal.
7. Le journal accepte ce type d'écriture (ex. une écriture du journal BQ1 mouvemente le compte de trésorerie associé).

### 6.3 Numérotation
- Format : `{journal}-{exercice}-{séquence sur 6 chiffres}` (ex. `AC-2026-000231`).
- Séquence continue sans trou par journal et par exercice, attribuée dans la même transaction que la validation (verrou sur la séquence).

### 6.4 Chaîne de hachage
- Pour chaque dossier : `hash_n = SHA-256(hash_{n-1} ‖ contenu canonique de l'écriture n)`. Le contenu canonique est une sérialisation déterministe (ordre des champs, format des montants, lignes triées par ordre).
- Le hash de tête est horodaté par une autorité externe (RFC 3161) chaque jour et à chaque clôture.
- **Outil de vérification** (binaire open source) : à partir d'un export, recalcule la chaîne et confirme l'intégrité.
- Écran « Intégrité » dans le dossier : état de la chaîne, derniers horodatages, bouton de vérification.

### 6.5 Pièces justificatives
- Stockées une fois, hachées (SHA-256), immuables (WORM).
- Relation **N-N** entre pièces et écritures (une écriture de banque peut s'appuyer sur le relevé **et** sur la facture ; un relevé alimente des dizaines d'écritures).
- Rôle de la pièce dans l'écriture : principale / complémentaire.

### 6.6 Écritures en devises
- Ligne : montant devise, devise, cours, contre-valeur XOF (arrondie au franc).
- Différence de règlement : écart de change en 676 (perte) / 776 (gain) — **à valider** (§26).
- Clôture : écarts de conversion (478 actif / 479 passif) et provision pour perte de change — **à valider** (§26).

---

## 7. Saisie et import

### 7.1 Saisie manuelle
- Grille au clavier : date, compte (recherche par numéro ou libellé), tiers, libellé, débit, crédit ; ligne d'équilibre proposée automatiquement.
- Mode « saisie par pièce » (en-tête + lignes) et mode « saisie au kilomètre » (journal par journal).
- Calcul automatique de la TVA depuis un montant TTC ou HT et un taux.
- Raccourcis : `Entrée` ligne suivante, `=` solde automatique, `F2` recherche de compte, `Ctrl+Entrée` mettre en « prête ».

### 7.2 Modèles d'écritures
Écriture type paramétrable (comptes, ventilation en %, libellé) appliquée en un clic : loyer, abonnement, échéance d'emprunt (avec ventilation capital/intérêts saisie).

### 7.3 Abonnements (écritures récurrentes)
- Création explicite par l'utilisateur, ou **suggestion** quand trois écritures similaires (même tiers, même compte, montant ± 5 %) sont détectées.
- À chaque échéance, l'écriture est générée **dans le brouillard** ; jamais validée automatiquement.

### 7.4 Import
- CSV/Excel d'écritures via un profil d'import (même moteur de profils que l'export, §18.3).
- FEC (18 colonnes).
- Toute écriture importée arrive dans le brouillard, sauf la reprise d'historique (§18.1).

---

## 8. Pipeline IA

### 8.1 Étapes
```
Pièce reçue (portail, e-mail, téléversement cabinet)
  │  hash SHA-256 → doublon exact → refus avec message
  ▼
Worker en bac à sable : type réel, antivirus, limites, rasterisation PDF → images
  ▼
Classification : type de document, qualité (lisible / illisible / incomplet)
  ▼
Extraction : champs structurés (schéma imposé)
  ▼
Contrôles déterministes (§8.4)
  ▼
Déduplication et rapprochement (§9)
  ▼
Imputation : règle validée > historique du tiers > IA guidée par le plan du dossier
  ▼
Construction de l'écriture par le moteur (montants issus de l'extraction)
  ▼
Score de confiance calculé + alertes → BROUILLARD, statut proposee_ia
```

### 8.2 Documents v1
| Type | Écriture produite |
|---|---|
| Facture d'achat | Journal AC : charge ou immobilisation, TVA récupérable, fournisseur |
| Facture de vente | Journal VT : client, produit, TVA collectée |
| Reçu / ticket | Journal CA ou trésorerie : charge, TVA si mentionnée, trésorerie |
| Relevé bancaire (PDF, image, CSV) | Mouvements de trésorerie (§9), écritures de banque |
| Relevé mobile money (Wave, Orange Money) | Mouvements de trésorerie, frais de l'opérateur |
| Avis de virement (émis, reçu) | Pièce complémentaire d'un mouvement de trésorerie |
| Note de frais | Journal OD : charges, TVA si récupérable, dette envers le salarié |
| Récapitulatif de paie | Journal PA : écriture de paie (§12) |

Les relevés CSV sont analysés sans IA (profil par banque ou opérateur). Contrôle systématique : solde initial + ∑mouvements = solde final.

### 8.3 Champs extraits
| Type | Champs |
|---|---|
| Facture | émetteur, destinataire, NINEA, n°, date, échéance, lignes (désignation, quantité, PU, HT, taux TVA), total HT, TVA par taux, TTC, retenues mentionnées, devise, mode de paiement |
| Reçu | commerçant, date, TTC, TVA, nature, moyen de paiement |
| Relevé | établissement, n° de compte, période, soldes, mouvements (date, date de valeur, libellé, montant, sens, solde) |
| Avis de virement | donneur d'ordre, bénéficiaire, montant, date, référence, motif |
| Note de frais | salarié, période, lignes (date, nature, montant, TVA, justificatif joint) |
| Récap. de paie | période, par rubrique : brut, cotisations salariales et patronales par organisme, impôts retenus, avances, net à payer |

### 8.4 Contrôles déterministes
| Contrôle | Règle | Effet si échec |
|---|---|---|
| Arithmétique | ∑lignes = HT ; HT + TVA = TTC (tolérance 1 XOF) | Bloquant |
| Taux de TVA | Taux ∈ paramètres du pays | Alerte |
| Date | Dans un exercice ouvert, pas dans le futur | Alerte |
| NINEA | Format | Alerte |
| Devise | ≠ XOF → cours requis | Bloquant |
| Doublon logique | Même émetteur + même n° + même montant | Fusion proposée (§9) |
| Relevé | Solde initial + mouvements = solde final | Bloquant |

### 8.5 Imputation
1. **Règle validée** du dossier (tiers → compte, mot-clé → compte, avec journal et ventilation TVA).
2. **Historique** : écritures validées du même tiers (y compris reprises de l'historique, §18.1), utilisées comme exemples.
3. **IA guidée** : le modèle reçoit l'extraction, les comptes candidats du plan du dossier (filtrés par tronc plausible) et les exemples ; il **choisit parmi les candidats** et renvoie : comptes par ligne, journal, ventilation TVA, justification courte, alertes.

Alertes possibles : `immobilisation_probable` (montant ≥ seuil et bien durable), `cca_probable` (période couverte au-delà de l'exercice), `tva_non_recuperable_possible`, `tiers_nouveau`, `compte_a_creer`, `devise`, `retenue_a_la_source`.

### 8.6 Écritures types (défauts Sénégal, modifiables par dossier)
| Opération | Écriture |
|---|---|
| Achat de marchandises | D 601x · D 4452 / C 401x |
| Achat de matières et fournitures | D 602x/604x/605x · D 4452 / C 401x |
| Transport | D 61x · D 4453 / C 401x |
| Services extérieurs | D 62x/63x · D 4454 / C 401x |
| Immobilisation | D 2xxx · D 4451 / C 481x |
| Vente de marchandises | D 411x / C 701x · C 4431 |
| Prestations de services | D 411x / C 706x · C 4431 |
| Retenue à la source opérée | Ligne au crédit de 447x (compte selon le type de retenue) |
| Règlement fournisseur | D 401x / C 521x (ou 571, ou compte mobile money) |
| Encaissement client | D 521x / C 411x |
| Frais bancaires | D 631x / C 521x |
| Frais d'opérateur mobile money | D 6317 / C 552x |
| Prélèvement de 0,5 % sur paiement marchand reçu (art. 220 ter CGI) | Montant brut au crédit du client ou du produit ; prélèvement au débit d'un compte paramétré (à valider, §26) |
| Virement interne (banque ↔ mobile money ↔ caisse) | Via 585 Virements de fonds |

### 8.7 Score de confiance
Calculé, jamais déclaré par le modèle :

| Composante | Effet |
|---|---|
| Contrôles bloquants tous passés | Requis, sinon niveau « faible » |
| Comptes tous existants | Requis |
| Règle validée appliquée | Fort bonus |
| Imputation identique aux 3 dernières écritures du tiers | Fort bonus |
| Tiers connu | Bonus |
| Aucune alerte | Bonus |
| Qualité d'image | Modulation |

Niveaux : **élevé** (éligible à la validation par lot), **moyen**, **faible**. Seuils calibrés sur les évaluations et révisés à partir du taux de correction observé par niveau.

### 8.8 Règles apprises
- Après 3 corrections identiques (paramètre dédié), une règle est **proposée**.
- Active uniquement après validation par un comptable.
- Suspendue automatiquement si elle est corrigée dans plus de 20 % des applications sur les 30 derniers jours.

### 8.9 Sécurité IA
Contenu des documents transmis comme données délimitées ; sorties validées par schéma ; comptes hors plan rejetés ; montants recalculés ; aucun droit d'écriture hors brouillard ; jeu d'évaluations anti-injection (§22).

### 8.10 Coût IA
Mesuré par pièce et par dossier (jetons, appels). Affiché à l'administrateur du cabinet. Quotas par offre, dépassement facturé à l'unité.

---

## 9. Trésorerie, relevés et déduplication

### 9.1 Comptes de trésorerie
Chaque compte bancaire, portefeuille mobile money et caisse est déclaré avec : compte comptable (sous-compte de 521 pour une banque, de 571 pour une caisse, de **552 « Monnaie électronique – téléphone portable »** pour un portefeuille mobile money (Wave, Orange Money…), un sous-compte par portefeuille — liste officielle des comptes et Guide d'application, chapitre 4, section 4), journal, identifiant (RIB, numéro de portefeuille), devise.

### 9.2 Engagements et mouvements
- **Engagement** : crée une créance ou une dette (facture). Journaux AC, VT, OD.
- **Mouvement de trésorerie** : fait bouger l'argent (ligne de relevé, reçu payé comptant). Journaux de trésorerie.
- Une facture et son règlement = deux écritures légitimes. Un **doublon** = deux documents décrivant le même engagement ou le même mouvement.

### 9.3 Règles de déduplication
| Situation | Règle |
|---|---|
| Même fichier reçu deux fois | Hash identique → refus |
| Même facture sous deux formes (photo + PDF) | Même émetteur + n° + montant → fusion proposée : une écriture, deux pièces |
| Avis de virement + ligne de relevé | Une seule écriture de trésorerie ; l'avis devient pièce complémentaire |
| Avis de virement reçu avant le relevé | Mouvement « provisoire » ; confirmé au rapprochement avec le relevé, sans nouvelle écriture |
| Reçu + facture d'un achat payé comptant | Écriture d'achat (facture) + écriture de caisse (reçu comme preuve du règlement) — jamais deux achats |
| Virement banque → Wave | Une écriture par relevé, chacune contre 585 ; alerte si 585 ne revient pas à zéro |
| Même relevé importé deux fois | Lignes identiques ignorées |

### 9.4 Rapprochement mouvement ↔ engagement
1. Référence de facture présente dans le libellé → **certain**.
2. Même montant + même tiers + date ± 30 jours → **probable**.
3. Montant ± 1 % ou écart égal aux frais de l'opérateur + même tiers → **possible**.
4. Aucun candidat → **à justifier**.

Le rapprochement produit une écriture de règlement **dans le brouillard**, et une proposition de lettrage (§10).

### 9.5 Rapprochement bancaire
- Pointage des lignes de relevé avec les écritures du compte de trésorerie.
- **État de rapprochement** à une date : solde relevé, solde comptable, écritures non pointées de part et d'autre, écart (doit être nul).
- État de rapprochement figé et archivé à chaque fin de mois et à la clôture.

### 9.6 Mouvements à justifier
Tableau par dossier et par période ; demande groupée au client via le portail (§19) ; réponse par pièce ou par texte libre ; une réponse sans pièce produit une écriture marquée « sans pièce » que le comptable doit valider explicitement.

---

## 10. Tiers, lettrage et échéancier

### 10.1 Lettrage
- Sur les comptes lettrables (tiers ; 585 ; comptes d'attente).
- Code de lettrage par compte ; lettrage **total** (solde nul) ou **partiel** (acompte, règlement partiel).
- Lettrage automatique **proposé** : même référence > même montant exact > combinaison de montants (somme de factures = règlement). Validé par l'utilisateur ou appliqué automatiquement selon un paramètre du dossier (désactivé par défaut).
- Délettrage possible tant que l'exercice n'est pas clôturé ; journalisé.

### 10.2 Échéancier
- Échéance créée à la validation d'une facture (date de la pièce + conditions de paiement du tiers, ou date lue sur la facture).
- Statut calculé depuis le lettrage : ouverte, partiellement réglée, soldée.

### 10.3 Éditions tiers
Balance auxiliaire, grand livre auxiliaire, **balance âgée** (non échu, 0–30, 31–60, 61–90, > 90 jours), relevé de compte d'un tiers (PDF).

---

## 11. TVA

### 11.1 Calcul par période
- TVA collectée = mouvements créditeurs nets de 4431 sur la période.
- TVA récupérable = mouvements débiteurs nets de 4451 + 4452 + 4453 + 4454 (+ comptes de régularisation paramétrés).
- TVA due ou crédit = différence, en tenant compte du crédit reporté de la période précédente (4449).
- Détail par taux et par écriture, avec lien vers les pièces.

### 11.2 Liquidation
L'assistant génère l'écriture de liquidation (brouillard, journal OD) :
```
D 4431   TVA collectée
   C 4451/4452/4453/4454   TVA récupérable
   C 4441   TVA due              (ou D 4449 crédit de TVA à reporter)
```
Le paiement suit : D 4441 / C 521x.

### 11.3 Verrouillage
Après validation de la liquidation, la période est **verrouillée** (§4.3). Une pièce de la période arrivée en retard est comptabilisée sur la période ouverte suivante, ou la période est déverrouillée avec motif.

### 11.4 Hors v1
Formulaire officiel et télédéclaration. Sigma Books fournit l'état récapitulatif à reporter.

---

## 12. Paie et notes de frais

### 12.1 Paie
Sigma Books ne calcule pas la paie. Il comptabilise un **récapitulatif de paie** (lu par l'IA ou importé en CSV) :
```
D 661x/663x…  Rémunérations (brut, par rubrique)
D 664x        Charges sociales patronales
   C 422      Personnel, rémunérations dues (net à payer)
   C 431x…    Organismes sociaux (part salariale + patronale)
   C 447x     Impôts retenus sur salaires
   C 421x     Avances et acomptes récupérés
```
Comptes exacts par rubrique paramétrés par le cabinet (table de correspondance rubrique → compte), à valider (§26).

### 12.2 Notes de frais
```
D 6xx         Charges (par nature)
D 445x        TVA récupérable (si justificatif conforme)
   C (compte de dette envers le salarié, paramétré par le cabinet)
```
Si une avance avait été versée : imputation sur 4211/4213. Remboursement : D compte de dette / C trésorerie.

---

## 13. Immobilisations

### 13.1 Registre
Désignation, catégorie, compte d'immobilisation (2xxx), compte d'amortissement (28xx), compte de dotation (681x), date d'acquisition, date de mise en service, valeur d'origine HT, valeur résiduelle, durée, méthode, pièce d'acquisition, localisation, statut (en service, cédée, mise au rebut).

- Création depuis une écriture d'achat portant l'alerte `immobilisation_probable` (pré-remplie), ou manuellement.
- Catégories avec durées et méthodes par défaut paramétrables.

### 13.2 Linéaire
- Annuité = (valeur d'origine − valeur résiduelle) / durée.
- Pro rata temporis en jours à partir de la date de mise en service, et jusqu'à la date de sortie.
- Dernière annuité = solde exact (pas d'arrondi résiduel).

### 13.3 Dégressif
- Taux = taux linéaire × coefficient ; base = valeur nette comptable.
- Pro rata en mois à partir du mois d'acquisition (règle usuelle — à valider).
- Bascule vers le linéaire quand l'annuité linéaire sur la durée restante devient supérieure.
- **Coefficients : paramètre fiscal du pays, à sourcer dans le CGI** ; aucune valeur codée en dur.

### 13.4 Dotations
L'assistant calcule les dotations de l'exercice pour toutes les immobilisations en service et génère une écriture d'inventaire (journal IN, brouillard) : D 681x / C 28xx, ventilée par compte.

### 13.5 Cession
```
1. Dotation complémentaire jusqu'à la date de cession       D 681x / C 28xx
2. Sortie de l'actif                                          D 28xx (amortissements cumulés)
                                                              D 812x (VNC)
                                                                 C 2xxx (valeur d'origine)
3. Prix de cession (toujours, en totalité)                    D 485x (ou trésorerie)
                                                                 C 822x
```
Résultat de cession = 822x − 812x. Les cessions courantes (654/754) sont gérées si la catégorie l'indique.

### 13.6 Mise au rebut
Dotation jusqu'à la date de sortie, puis sortie de l'actif : VNC en 81x, sans produit.

### 13.7 Éditions
Tableau des immobilisations, plan d'amortissement par bien, tableau des dotations, données des notes 3A, 3C et 3D.

---

## 14. Travaux de fin d'exercice

Un **assistant de fin d'exercice** guide le comptable. Chaque étape génère des écritures dans le journal IN (brouillard), avec un état de calcul joint comme pièce.

### 14.1 Stocks (inventaire intermittent)
- Saisie de la valeur du stock final par catégorie (marchandises, matières, fournitures, produits).
```
Annulation du stock initial :   D 603x / C 3x
Constatation du stock final :   D 3x   / C 603x
```
(Pour les stocks de produits : variation en 73x.) Dépréciation de stock si nécessaire (§14.5).

### 14.2 Charges à payer et factures non parvenues
Liste des charges connues non facturées : D 6xx · D 4455 / C 408x (ou 4286 et comptes de dettes sociales et fiscales à payer).

### 14.3 Produits à recevoir et factures à établir
D 4181 / C 7xx · C (compte de TVA sur factures à établir, paramétré).

### 14.4 Charges et produits constatés d'avance
- Proposés à partir des alertes `cca_probable` de l'exercice : D 476 / C 6xx ; D 7xx / C 477.
- Calcul au prorata des jours.

### 14.5 Provisions et dépréciations
Saisie assistée : provisions pour risques et charges (D 691x / C 19x), dépréciations d'actifs (créances, stocks), reprises (79x). Pour les créances : proposition à partir de la balance âgée.

### 14.6 Contre-passation à l'ouverture
Les écritures de régularisation (408, 418, 476, 477, 4286…) sont **contre-passées automatiquement** au premier jour de l'exercice suivant (proposées dans le brouillard de N+1).

### 14.7 Opérations en devises
Réévaluation des créances et dettes en devises au cours de clôture ; écarts de conversion actif/passif ; provision pour perte de change (§6.6, à valider).

### 14.8 Impôt sur le résultat
Saisie assistée du montant de l'impôt (calcul fiscal hors v1) : D 891 / C 441. Contrôle de l'impôt minimum forfaitaire à paramétrer (à sourcer).

---

## 15. Clôture et réouverture

### 15.1 Liste de contrôle (bloquante)
| Contrôle | Condition |
|---|---|
| Brouillard | Aucune écriture non validée sur l'exercice |
| Balance | Équilibrée |
| Chaîne d'intégrité | Vérifiée sans erreur |
| Rapprochements bancaires | Un état de rapprochement au dernier jour de l'exercice pour chaque compte de trésorerie, écart nul |
| Comptes de virements internes | 585 soldé |
| Comptes d'attente | Soldés (comptes d'attente désignés par le cabinet, par ex. sous-comptes de 471) |
| TVA | Toutes les périodes liquidées |
| Immobilisations | Dotations de l'exercice validées |
| Fin d'exercice | Étapes de l'assistant toutes « faites » ou « sans objet » (avec justification) |
| États financiers | Bilan équilibré (actif = passif) ; résultat du Bilan = résultat du Compte de résultat ; TFT : trésorerie de clôture = trésorerie du Bilan |
| Notes annexes | Toutes les notes applicables renseignées |

### 15.2 Clôture
- Action réservée au rôle habilité, avec ré-authentification et confirmation explicite (« irréversible »).
- Effets : exercice `cloture`, périodes `cloturee`, horodatage externe du hash de tête, archivage des états financiers (PDF + Excel + FEC de l'exercice + hash), génération des à-nouveaux.

### 15.3 À-nouveaux
- Journal AN de N+1 : reprise des soldes des comptes de **bilan** (classes 1 à 5), détaillés par tiers pour les comptes auxiliaires (les lettrages ouverts restent rapprochables).
- Résultat de l'exercice repris sur le compte 131 (bénéfice) ou 139 (perte), ou 130 « résultat en instance d'affectation » selon le paramètre du dossier (à valider, §26).
- Les à-nouveaux sont générés et validés automatiquement (seule exception à la validation humaine, car ce sont des reprises mécaniques d'un exercice clôturé et vérifié).

### 15.4 Erreur découverte après clôture
Pas de réouverture. Correction dans l'exercice en cours (écriture sur exercice antérieur, mention en annexe).

---

## 16. Répartition du résultat

Assistant déclenché après l'assemblée générale (date de l'AGO obligatoire).

### 16.1 Bénéfice
1. **Données** : résultat net (131), report à nouveau antérieur (121 créditeur ou 129 débiteur), capital, réserve légale existante (111).
2. **Réserve légale** (sociétés concernées par la forme juridique) : dotation minimale = 10 % du **bénéfice diminué des pertes antérieures**, jusqu'à ce que la réserve atteigne 20 % du capital (AUSCGIE — référence d'article à vérifier, §26).
3. **Bénéfice distribuable** = bénéfice − pertes antérieures − dotation à la réserve légale + report à nouveau créditeur antérieur.
4. Affectations saisies : réserves statutaires (112), autres réserves dont facultatives (118), dividendes (465), report à nouveau (121), autres (tiers créditeur paramétré).
5. Contrôle : ∑affectations = résultat + report à nouveau antérieur ; dotation légale ≥ minimum.

```
D 131   Résultat net : bénéfice
D 121   Report à nouveau créditeur (antérieur, si affecté)
   C 111   Réserve légale
   C 112   Réserves statutaires
   C 118   Autres réserves
   C 465   Associés, dividendes à payer
   C 121   Report à nouveau créditeur
```

### 16.2 Perte
Par défaut : D 129 / C 139. Toute imputation sur des réserves est un choix explicite saisi dans l'assistant (décision de l'AG).

### 16.3 Note 31
Données de la répartition reprises dans la note correspondante.

---

## 17. États financiers et éditions

### 17.1 Éditions courantes
Journaux, grand livre (général et auxiliaire), balance générale (2, 4, 6 ou 8 colonnes), balance auxiliaire, balance âgée, centralisateur des journaux, état de rapprochement, tableau des immobilisations, état de TVA.

Chaque édition : filtres (période, comptes, journaux, tiers), mode « provisoire » (avec brouillard) ou « définitif » (validé seulement), export PDF et Excel.

### 17.2 États financiers SYSCOHADA (Système Normal)
- **Bilan** : actif (brut, amortissements et dépréciations, net, net N-1) et passif (net, net N-1), postes AD à BZ et CA à DZ.
- **Compte de résultat** : postes TA à XI, soldes XA à XI, N et N-1.
- **Tableau des flux de trésorerie** : postes ZA à ZH, méthode CAFG.
- **Notes annexes** : liste officielle, chaque note marquée applicable / non applicable ; notes calculées automatiquement depuis les écritures et le registre des immobilisations ; notes narratives saisies.
- Page de garde et fiche d'identification.

### 17.3 Moteur de correspondance
- Les états sont décrits par des **fichiers de correspondance versionnés** (poste → liste de troncs de comptes, sens du solde, colonne). Aucune correspondance n'est codée en dur.
- Un compte à solde inversé est reclassé selon les règles des tableaux de correspondance officiels (ex. banque créditrice au passif).
- Source : tableaux de correspondance AUDCIF du Bilan et du Compte de résultat (dossier `syscohada/` du projet).

### 17.4 CAFG
Conformément au Guide d'application (§4.2.3.2) :
```
CAFG = EBE (XD)
       + valeurs comptables des cessions courantes (654)
       − produits des cessions courantes (754)
       + résultat financier (XF)
       + autres produits HAO (TO)
       − autres charges HAO (RP)
       − participation des travailleurs (RQ)
       − impôts sur le résultat (RS)
```
Test de non-régression : l'exemple chiffré du Guide (CAFG N = 241 773 904) doit être reproduit exactement.

### 17.5 Contrôles de cohérence affichés
Actif = passif ; résultat Bilan = résultat CdR ; trésorerie TFT = trésorerie Bilan ; total des notes = postes correspondants.

### 17.6 Production
PDF générés avec Typst sur des modèles fidèles aux présentations officielles ; Excel avec formules de totalisation ; archivage horodaté à la clôture.

---

## 18. Migration, interopérabilité et réversibilité

### 18.1 Reprise de données (entrée d'un dossier existant)
| Source | Contenu repris |
|---|---|
| Plan de comptes du logiciel d'origine | Comptes et sous-comptes, avec rattachement au tronc SYSCOHADA (proposé automatiquement, validé par le comptable) |
| Fichier des tiers | Clients, fournisseurs, comptes auxiliaires |
| Balance d'ouverture | À-nouveaux de l'exercice de démarrage, détaillés par tiers |
| FEC d'exercices antérieurs (optionnel) | Historique en **lecture seule** (pour les comparatifs N-1 et l'apprentissage de l'imputation) |
| Registre des immobilisations | Biens, cumul des amortissements à la date de reprise |

Un rapport de reprise liste les anomalies (comptes sans tronc, balance déséquilibrée, tiers en double).

### 18.2 Export FEC
Export de tout ou partie d'un exercice au format FEC (18 colonnes), à tout moment. Sert aussi à l'audit et à la réversibilité.

### 18.3 Profils d'export et d'import
Moteur de profils déclaratifs (séparateur, encodage, format de date, représentation des montants, longueur des comptes, colonnes) avec préréglages : FEC, Sage 100 (modèle d'import paramétrable), Odoo, Excel/CSV générique ; assistant de création de profil à partir d'un fichier exemple avec test aller-retour.

### 18.4 Réversibilité
À la résiliation : export complet (FEC de tous les exercices, pièces, registre des immobilisations, états archivés, chaîne d'intégrité et outil de vérification), puis effacement cryptographique (Fondations §3.7).

---

## 19. Portail client

| Fonction | Détail |
|---|---|
| Dépôt de pièces | Photo (recadrage, redressement), PDF, plusieurs pages ; file d'envoi hors ligne |
| Relevés | Import CSV/PDF bancaire, Wave, Orange Money |
| E-mail de dépôt | Adresse unique par dossier |
| Demandes du cabinet | Mouvements à justifier, questions ; réponse par pièce ou texte |
| Suivi | Pièces envoyées, acceptées, refusées (motif) |
| Indicateurs (option activée par le cabinet) | Trésorerie, chiffre d'affaires, créances et dettes échues |

Le client ne voit jamais les écritures. Conception pour téléphones Android d'entrée de gamme et connexion intermittente.

---

## 20. Rôles et droits

Politiques Cedar, refus par défaut. Rôles par défaut (un cabinet peut créer des rôles dérivés, jamais plus larges que le rôle administrateur) :

| Action | Admin cabinet | Valideur | Collaborateur | Auditeur | Client |
|---|---|---|---|---|---|
| Gérer cabinet, utilisateurs, facturation | ✅ | ❌ | ❌ | ❌ | ❌ |
| Créer un dossier, reprise de données | ✅ | ✅ | ❌ | ❌ | ❌ |
| Paramétrer plan, journaux, fiscalité | ✅ | ✅ | ❌ | ❌ | ❌ |
| Déposer des pièces | ✅ | ✅ | ✅ | ❌ | ✅ (ses dossiers) |
| Saisir, corriger dans le brouillard | ✅ | ✅ | ✅ | ❌ | ❌ |
| Valider une écriture | ✅ | ✅ | ⚙️ selon délégation | ❌ | ❌ |
| Validation par lot | ✅ | ✅ | ❌ | ❌ | ❌ |
| Contre-passer | ✅ | ✅ | ❌ | ❌ | ❌ |
| Lettrer / délettrer | ✅ | ✅ | ✅ | ❌ | ❌ |
| Activer une règle apprise | ✅ | ✅ | ❌ | ❌ | ❌ |
| Liquider la TVA, verrouiller une période | ✅ | ✅ | ❌ | ❌ | ❌ |
| Déverrouiller une période | ✅ | ❌ | ❌ | ❌ | ❌ |
| Travaux de fin d'exercice | ✅ | ✅ | ✅ (brouillard) | ❌ | ❌ |
| Clôturer un exercice | ✅ | ✅ | ❌ | ❌ | ❌ |
| Consulter éditions et états | ✅ | ✅ | ✅ | ✅ | ❌ |
| Vérifier l'intégrité, consulter l'audit | ✅ | ✅ | ❌ | ✅ | ❌ |
| Exporter (FEC, profils) | ✅ | ✅ | ❌ | ✅ | ❌ |
| Répondre aux demandes | ❌ | ❌ | ❌ | ❌ | ✅ |

Séparation des tâches paramétrable : interdire qu'un utilisateur valide une écriture qu'il a lui-même saisie.

---

## 21. Exigences de sécurité produit

Le détail est dans les Fondations (§3). Exigences visibles côté produit :
- Passkeys ou TOTP obligatoires pour tous les rôles cabinet ; ré-authentification pour validation par lot, clôture, déverrouillage, export complet, modification des droits.
- Journal d'audit consultable (qui, quoi, quand, avant/après), exportable, chaîné.
- Écran Intégrité (§6.4).
- Sessions actives visibles et révocables par l'utilisateur et l'administrateur.
- Alertes de sécurité à l'administrateur : nouvelle connexion, export complet, changement de droits.
- Chaque requête de l'interface est autorisée côté serveur ; l'interface ne fait que masquer.

---

## 22. Évaluations et tests de conformité

### 22.1 Tests comptables (bloquants)
- **Tests de propriétés** du moteur : toute suite d'écritures valides produit une balance équilibrée ; une contre-passation annule exactement ; les à-nouveaux reproduisent la balance de clôture des comptes de bilan ; le lettrage total solde le compte du tiers.
- **Cas de référence** : exemples chiffrés du Guide d'application AUDCIF (états financiers, TFT, CAFG) reproduits exactement.
- **Scénarios portés** des suites de tests de projets open source (Fondations §6), avec attribution.
- Amortissements : linéaire, dégressif avec bascule, pro rata, cession, rebut.

### 22.2 Évaluations IA (seuils de mise en production)
| Domaine | Métrique | Seuil |
|---|---|---|
| Extraction | Champs corrects | ≥ 95 % |
| Extraction | Montants corrects | ≥ 98 % |
| Imputation | Compte correct au tronc | ≥ 85 % |
| Imputation | Ventilation TVA correcte | ≥ 95 % |
| Imputation | Journal correct | ≥ 95 % |
| Équilibre | Écritures équilibrées | 100 % |
| Déduplication | Doublons non détectés | ≤ 1 % |
| Déduplication | Fusions à tort | ≤ 2 % |
| Sécurité | Injections sans effet | ≥ 99 % |
| Confiance | Taux de correction au niveau « élevé » | ≤ 3 % |

Toute modification de prompt, de modèle ou de règle relance la suite ; une régression bloque la mise en production. Les jeux de données réels restent privés (Fondations §7).

---

## 23. Modèle de données (entités principales)

```
Cabinet, Utilisateur, Appartenance(utilisateur, cabinet, rôle), AccesDossier, Passkey, Session

Dossier            ── forme juridique, NINEA, RCCM, pays, devise, paramètres
Exercice           ── dates, statut, hash de clôture
Periode            ── mois, statut, verrouillée_par, motif
ParametresFiscaux  ── pays, version, contenu
Compte             ── numéro, libellé, tronc, classe, nature, sens, collectif, lettrable, actif
Journal            ── code, libellé, type, compte de trésorerie associé, séquence
Tiers              ── identité, type, compte collectif, compte auxiliaire, conditions
CompteTresorerie   ── type, identifiant, compte, journal, devise
CoursDevise        ── devise, date, cours

Piece              ── sha256, stockage, source, type, qualité, statut
Extraction         ── pièce, version du modèle, données, contrôles, coût
MouvementTresorerie── compte, date, montant, sens, libellé, statut, empreinte

Ecriture           ── dossier, exercice, journal, date, n° définitif, libellé, statut,
                      origine (ia/manuel/import/modèle/abonnement/inventaire/an/liquidation),
                      confiance, alertes, contre-passe_id, hash, hash_precedent,
                      validée_par, validée_le, version paramètres
LigneEcriture      ── compte, tiers, libellé, débit, crédit, devise, montant devise, cours
PieceEcriture      ── pièce, écriture, rôle                       (N-N)
Lettrage           ── compte, code, lignes, total/partiel, date
Echeance           ── ligne d'origine, date, montant, statut
Rapprochement      ── mouvement, écriture, niveau, statut
EtatRapprochement  ── compte de trésorerie, date, soldes, éléments non pointés, figé

Immobilisation, LigneAmortissement, Cession
TravailFinExercice ── exercice, étape, statut, écritures générées, justification
RepartitionResultat
DeclarationTVA     ── période, montants, écriture de liquidation, statut
NoteAnnexe         ── exercice, code, applicable, contenu calculé, texte
ArchiveEtats       ── exercice, fichiers, hashes, horodatage

ModeleEcriture, Abonnement, RegleImputation, Correction
ProfilImportExport, LotExport
DemandeClient
JournalAudit       ── append-only, chaîné
HorodatageExterne  ── dossier, hash, jeton RFC 3161, date
```

Montants : décimal exact (`NUMERIC` en base, `rust_decimal` dans le moteur), jamais de flottant.

---

## 24. Plan de développement

Lots séquencés selon les dépendances. Durées indicatives pour une petite équipe assistée d'agents de code ; plusieurs lots peuvent avancer en parallèle une fois le noyau stable.

| Lot | Contenu | Durée | Critère de fin |
|---|---|---|---|
| **L0 — Fondations** | Dépôt GitHub public, CI sécurisée, ADR, modèle de menaces, SECURITY.md, CLA, workspace Rust, PostgreSQL, squelette SvelteKit | 2 sem. | Pipeline CI vert avec tous les contrôles des Fondations §8 |
| **L1 — Noyau comptable** | Référentiel SYSCOHADA, écritures, brouillard/validation, numérotation, périodes, exercices, contre-passation, chaîne de hachage, balance, grand livre, journaux | 6 sem. | Tests de propriétés + outil de vérification d'intégrité |
| **L2 — Socle applicatif** | Passkeys, sessions, Cedar, RLS, audit, multi-tenant, chiffrement par tenant, saisie manuelle, éditions de base | 5 sem. | Tests d'isolation entre tenants ; premier dossier tenu à la main de bout en bout |
| **L3 — Tiers et trésorerie** | Tiers, lettrage, échéancier, balance âgée, comptes de trésorerie, import de relevés (CSV), rapprochement bancaire | 5 sem. | État de rapprochement à écart nul sur jeu de test |
| **L4 — IA et portail** | Worker en bac à sable, pipeline IA, déduplication, file de validation, règles apprises, portail client, évaluations | 7 sem. | Seuils du §22.2 atteints |
| **L5 — TVA, paie, notes de frais** | Calcul et liquidation de TVA, verrouillage, paie depuis récapitulatif, notes de frais, modèles et abonnements | 3 sem. | Scénarios TVA sur 12 mois |
| **L6 — Fin d'exercice et clôture** | Immobilisations, assistant de fin d'exercice, devises, clôture, à-nouveaux, répartition du résultat | 6 sem. | Exercice complet clôturé et rouvert sur jeu de test |
| **L7 — États financiers** | Moteur de correspondance, Bilan, CdR, TFT, notes, PDF Typst, Excel, archivage | 5 sem. | Exemples du Guide AUDCIF reproduits exactement |
| **L8 — Interopérabilité** | Reprise de données, FEC import/export, profils et préréglages, réversibilité | 3 sem. | Aller-retour FEC sans perte |
| **L9 — Durcissement** | Pentest externe, bug bounty AarMe, performance, documentation utilisateur, sauvegarde/restauration testée | 4 sem. | Aucune vulnérabilité critique ou haute ouverte |

Total séquentiel : ~46 semaines. Avec parallélisation (L3, L5 et L8 en parallèle de L4 ; L7 en parallèle de L6) : **environ 30 à 34 semaines**.

---

## 25. Après la v1

| Horizon | Contenu |
|---|---|
| v1.1 | Connecteurs API (Odoo, ERPNext, Dolibarr, Tryton) ; dépôt par WhatsApp ; Système Minimal de Trésorerie |
| v1.2 | Côte d'Ivoire puis autres pays UEMOA ; analytique légère |
| v2 | Télédéclaration (si les portails le permettent) ; trésorerie prévisionnelle ; offre directe aux TPE avec relecture par un cabinet partenaire |

---

## 26. Points à faire valider par un expert-comptable

Ces points touchent à l'exactitude comptable et fiscale. Ils doivent être confirmés avant la mise en production du lot concerné ; les valeurs restent des paramètres, jamais du code en dur.

| # | Point | Lot |
|---|---|---|
| 1 | Moment du caractère définitif : validation (choix retenu) ou clôture de période | L1 |
| 2 | Comptabilisation du prélèvement de 0,5 % sur paiements marchands (art. 220 ter CGI) et de la taxe sur les transferts d'argent (art. 395 s.) | L3 |
| 3 | Taux réduits et exonérations de TVA ; types, taux et comptes des retenues à la source (CGI Sénégal) | L5 |
| 4 | Comptes de paie par rubrique ; compte de dette des notes de frais | L5 |
| 5 | Coefficients et règles de pro rata du dégressif (CGI Sénégal) | L6 |
| 6 | Comptes d'écarts de change et de conversion ; provision pour perte de change | L6 |
| 7 | Compte de reprise du résultat à l'ouverture (130, 131/139) | L6 |
| 8 | Référence d'article et champ d'application de la réserve légale (AUSCGIE) | L6 |
| 9 | Durée minimale et maximale d'un premier exercice | L1 |
| 10 | Impôt minimum forfaitaire et modalités de l'impôt sur le résultat | L6 |
| 11 | Reclassements des soldes inversés selon les tableaux de correspondance | L7 |

---

*Sigma Books — Spécification fonctionnelle v1.0 — Logiciel comptable SYSCOHADA complet, piloté par l'IA — Novembre 2026*
