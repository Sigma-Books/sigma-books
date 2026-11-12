# Sigma Books — Fondations techniques
## Langage, architecture « security by design », licence open source, briques réutilisables, modèle de déploiement

> **Statut :** décisions validées — novembre 2026
> **Spécification fonctionnelle associée :** *Sigma Books — Spécification fonctionnelle v1.0* (logiciel comptable complet dès la v1)
> **Hypothèses de départ :**
> - Projet démarré de zéro : aucun choix technique n'est imposé par l'existant.
> - Sigma Books est un logiciel comptable complet (SYSCOHADA), avec la couche IA de lecture des pièces.
> - Le code source sera **public sur GitHub**.
> - La sécurité est une exigence de conception, pas une couche ajoutée.
>
> Avertissement : les points de licence ne constituent pas un avis juridique.

---

## Sommaire
1. Décisions (synthèse)
2. Le langage
3. Architecture « security by design »
4. Licence et modèle open source
5. Modèle de déploiement
6. Briques open source réutilisables
7. Organisation du dépôt GitHub
8. Cycle de développement sécurisé
9. Feuille de route
10. Décisions validées

---

## 1. Décisions (synthèse)

| Sujet | Décision | Raison principale |
|---|---|---|
| Langage du backend et du moteur comptable | **Rust** | Sûreté mémoire et de concurrence garanties à la compilation, système de types qui permet d'encoder les règles comptables, chaîne d'approvisionnement outillée |
| Interface web de l'application | **TypeScript strict + SvelteKit** en mode statique, servi par le backend Rust | Le navigateur n'exécute que du JS/WASM ; Svelte réduit le nombre de dépendances ; aucun serveur Node en production |
| Site public et documentation | **Astro** (pages statiques, zéro JS par défaut) | C'est exactement le cas d'usage d'Astro |
| Architecture | **Monolithe modulaire** + **workers isolés** pour les documents non fiables | Moins de composants = moins de surface d'attaque ; isolation forte là où le risque est réel |
| Base de données | **PostgreSQL** (RLS, rôles séparés, tables comptables en ajout seul) | Maturité, intégrité transactionnelle, sécurité par ligne |
| Autorisation | **Cedar** (moteur de politiques en Rust, Apache-2.0), refus par défaut | Politiques lisibles, vérifiables et testables, séparées du code |
| Authentification | **Passkeys (WebAuthn)** en principal, TOTP en secours, jamais de SMS | Résistance au phishing |
| Intégrité comptable | Chaîne de hachage par dossier + horodatage externe + outil de vérification public | Preuve d'inaltérabilité vérifiable par un auditeur |
| Licence | **AGPL-3.0-or-later** pour tout le code | Empêche un tiers de revendre Sigma Books en SaaS sans contribuer ; permet de reprendre légalement du code GPL-3, LGPL-3 et AGPL |
| Marque | « Sigma Books » et le logo déposés (OAPI) et **non** couverts par la licence | Le code est libre, le nom ne l'est pas (modèle Odoo) |
| Déploiement | **SaaS multi-tenant d'abord** ; auto-hébergement possible (communauté) ; instance dédiée gérée ensuite | Modèle des éditeurs open source qui réussissent (Odoo, Frappe) |
| Contributions externes | **CLA** (accord de licence de contribution) | Conserve la possibilité d'une double licence commerciale |

---

## 2. Le langage

### 2.1 Une précision sur Astro
Astro n'est pas un langage : c'est un **framework web JavaScript/TypeScript** pour construire des sites de contenu (sites vitrines, blogs, documentation) rapides et en grande partie statiques. Il est très bien adapté à un site public, et pas du tout au cœur d'un logiciel comptable. Dans cette architecture, Astro est retenu **pour le site public et la documentation** de Sigma Books.

### 2.2 Critères de choix
1. **Sûreté mémoire** : les agences de cybersécurité (NSA, CISA et leurs partenaires) recommandent de développer les nouveaux logiciels dans des langages sûrs pour la mémoire, et de publier une feuille de route de sortie du C/C++.
2. **Expressivité du système de types** : peut-on rendre impossibles, à la compilation, des états comptables invalides ?
3. **Chaîne d'approvisionnement** : audit des dépendances, licences, provenance.
4. **Surface d'attaque à l'exécution** : taille du runtime, de l'image, des dépendances système.
5. **Écosystème** : web, PostgreSQL, cryptographie, génération de PDF.
6. **Productivité et recrutement**, y compris avec des agents de code IA.

### 2.3 Comparaison

| Langage | Sûreté mémoire | Types (invariants métier) | Chaîne d'appro. | Runtime | Écosystème | Recrutement |
|---|---|---|---|---|---|---|
| **Rust** | ✅ garantie à la compilation, y compris en concurrence | ✅✅ types algébriques, pas de `null`, erreurs explicites (`Result`), énumérations exhaustives | ✅✅ `cargo-audit`, `cargo-deny`, `cargo-vet` | ✅✅ binaire unique, image minimale | ✅ (axum, sqlx, rustls, Cedar, Typst) | ⚠️ plus rare, courbe d'apprentissage |
| **Go** | ✅ (ramasse-miettes) | ⚠️ types simples, `nil` possible | ✅ `govulncheck` | ✅ binaire unique | ✅ | ✅ |
| **C# / .NET** | ✅ | ✅ | ✅ | ⚠️ runtime lourd | ✅✅ | ✅ |
| **Java / Kotlin** | ✅ | ✅ (Kotlin) | ✅ | ⚠️ JVM | ✅✅ | ✅ |
| **TypeScript (Node)** | ✅ | ⚠️ typage effacé à l'exécution | ⚠️ écosystème npm très exposé aux attaques de dépendances | ⚠️ | ✅✅ | ✅✅ |
| **Python** | ✅ | ⚠️ typage optionnel | ⚠️ | ⚠️ | ✅ | ✅✅ |

### 2.4 Recommandation : Rust
Ce que Rust apporte concrètement à un logiciel comptable :

```rust
// Un montant n'est jamais un flottant : décimal exact, devise attachée.
pub struct Montant { valeur: Decimal, devise: Devise }

// Un numéro de compte ne peut pas exister s'il est invalide.
pub struct NumeroCompte(String);           // construit uniquement via NumeroCompte::parse()

// Une écriture équilibrée est un type différent d'une écriture brouillon.
pub struct EcritureBrouillon { /* … */ }
pub struct EcritureEquilibree { /* … */ }  // seule fonction de création : valider_equilibre()

// Seule une écriture équilibrée peut être enregistrée dans le journal.
fn enregistrer(e: EcritureEquilibree) -> Result<Enregistree, ErreurComptable>
```

L'équilibre débit/crédit, la validité des comptes ou l'interdiction de modifier une période close deviennent des **propriétés vérifiées par le compilateur**, en plus des tests.

**Contreparties assumées :**
- Courbe d'apprentissage et temps de compilation plus longs.
- Développeurs Rust plus rares au Sénégal. Les agents de code IA compensent en partie, mais les **revues de code humaines** restent obligatoires sur le moteur comptable.

**Alternative crédible : Go**, si la vitesse de développement et le recrutement priment. On perd une partie des garanties de types, pas la sûreté mémoire.

### 2.5 Le cas du navigateur
Le navigateur n'exécute que du JavaScript ou du WebAssembly. Les options :
- **TypeScript strict + SvelteKit** (recommandé) : compilé en fichiers statiques, servi par le backend Rust. Aucune logique métier dans le navigateur, peu de dépendances, CSP stricte et Trusted Types.
- Rust → WebAssembly (Leptos, Dioxus) : un seul langage, mais un écosystème encore jeune pour des écrans métier denses (grilles, raccourcis clavier).

---

## 3. Architecture « security by design »

### 3.1 Référentiels suivis
| Référentiel | Usage |
|---|---|
| CISA *Secure by Design* | Principes produit : MFA par défaut, pas de mot de passe par défaut, élimination de classes entières de vulnérabilités, transparence |
| OWASP ASVS (niveau 2 partout, niveau 3 pour l'authentification, l'autorisation et le moteur comptable) | Exigences vérifiables, reprises comme critères d'acceptation |
| NIST SSDF (SP 800-218) | Cycle de développement sécurisé |
| SLSA (niveau 3 visé) | Provenance et intégrité des builds |
| STRIDE + LINDDUN | Modélisation des menaces (sécurité + vie privée) |

### 3.2 Principes
1. **Refus par défaut** : tout accès est interdit sauf politique explicite.
2. **Défense en profondeur** : l'isolation multi-tenant est assurée à trois niveaux (politique Cedar, requête applicative, RLS PostgreSQL).
3. **Moindre privilège** : chaque composant a son rôle de base de données et ses identifiants, limités au strict nécessaire.
4. **Données non fiables isolées** : un document client est considéré comme hostile jusqu'à preuve du contraire.
5. **Intégrité vérifiable** : l'inaltérabilité des écritures se démontre, elle ne se promet pas.
6. **Moins de composants** : chaque service ajouté est une surface d'attaque. File de tâches dans PostgreSQL (`SKIP LOCKED`) plutôt qu'un Redis supplémentaire.
7. **Sécurité par transparence** : code public, modèle de menaces public, aucune sécurité par l'obscurité.

### 3.3 Zones de confiance

```
                   Internet
                      │
        ┌─────────────▼──────────────┐
        │  Reverse proxy / WAF       │  TLS 1.3, HSTS, limitation de débit
        └─────────────┬──────────────┘
                      │
┌─────────────────────▼──────────────────────────────┐   ZONE APPLICATIVE
│  Monolithe Rust (axum)                             │
│  ├─ authentification (passkeys, sessions)          │
│  ├─ autorisation (Cedar, refus par défaut)         │
│  ├─ moteur comptable (crate pure, sans I/O)        │
│  ├─ API + fichiers statiques de l'interface        │
│  └─ orchestrateur IA (aucun droit d'écriture       │
│     comptable : ne produit que des propositions)   │
└──────┬───────────────────────┬─────────────────────┘
       │                       │ messages signés via la file
┌──────▼──────────┐   ┌────────▼────────────────────────┐   ZONE NON FIABLE
│  PostgreSQL     │   │  Workers documents (bac à sable)│
│  RLS, rôles     │   │  gVisor/nsjail, seccomp,        │
│  séparés,       │   │  sans réseau, sans accès BDD,   │
│  tables en      │   │  système de fichiers éphémère : │
│  ajout seul     │   │  détection du type, antivirus,  │
└──────┬──────────┘   │  rasterisation PDF → images     │
       │              └─────────────────────────────────┘
┌──────▼──────────────────┐   ┌────────────────────────────┐
│ Stockage objet (pièces) │   │ Sortie vers l'API IA via   │   ZONE SORTANTE
│ verrouillage WORM,      │   │ proxy en liste blanche      │
│ chiffrement par tenant  │   └────────────────────────────┘
└─────────────────────────┘
```

### 3.4 Identité et sessions
- **Passkeys (WebAuthn)** comme méthode principale ; TOTP en secours ; **pas de SMS** (vulnérable au SIM swapping).
- MFA obligatoire pour tous les rôles cabinet, dès la création du compte.
- Sessions par cookie `HttpOnly`, `Secure`, `SameSite=Strict`, durée courte, rotation à chaque élévation de privilège.
- **Ré-authentification** pour les actions sensibles : validation par lot, export, clôture, modification des droits.
- Mots de passe (si utilisés) : Argon2id, contrôle contre les listes de mots de passe compromis.

### 3.5 Autorisation
- Politiques **Cedar** versionnées dans le dépôt, testées comme du code (tests unitaires des politiques + analyse automatique de leurs propriétés).
- Chaque requête porte un contexte `{cabinet, dossier, utilisateur, rôle}`. Le backend positionne ce contexte dans la transaction PostgreSQL (`SET LOCAL`), et les politiques RLS le vérifient.
- Séparation des tâches : celui qui saisit n'est pas forcément celui qui valide ; la clôture exige un rôle dédié.

### 3.6 Intégrité du grand livre
- Les tables comptables sont en **ajout seul** : le rôle applicatif n'a pas les droits `UPDATE` ni `DELETE` sur les écritures validées. Une correction est une nouvelle écriture (contre-passation).
- Chaque écriture validée porte le **hash de la précédente** dans son dossier (chaîne de hachage).
- Le hash de tête de chaque dossier est **horodaté régulièrement** par une autorité externe (RFC 3161).
- Un **outil de vérification open source** permet à un auditeur ou à un commissaire aux comptes de recalculer la chaîne à partir d'un export et de prouver qu'aucune écriture n'a été modifiée.

### 3.7 Données et chiffrement
- TLS 1.3 uniquement ; HSTS ; certificats automatisés.
- Chiffrement au repos par **clé de données par cabinet** (chiffrement d'enveloppe), clés maîtresses dans un KMS ou OpenBao.
- Suppression cryptographique : détruire la clé d'un cabinet rend ses données illisibles (utile à la résiliation).
- Pièces stockées en **WORM** (verrouillage d'objet), hachées à l'ingestion.

### 3.8 Documents non fiables
- Limites de taille et de nombre de pages ; détection du type réel (pas l'extension) ; antivirus (ClamAV).
- Les PDF sont **rasterisés en images dans le bac à sable**. Les bibliothèques de rendu PDF sont écrites en C/C++ : c'est précisément le code à confiner.
- Le worker n'a ni réseau ni accès à la base. Il reçoit un fichier et renvoie des images et des métadonnées.
- **Fuzzing** permanent des analyseurs maison (CSV bancaires, FEC, formats d'import).

### 3.9 IA
- L'orchestrateur IA **ne peut que proposer**. Il n'a aucune permission d'écriture comptable.
- Contenu des documents transmis comme données délimitées ; sorties validées par schéma ; comptes hors référentiel rejetés ; montants recalculés par le moteur.
- Minimisation : seules les images de la pièce et le contexte strictement nécessaire sont envoyés.
- Contractuel : pas d'entraînement des modèles du fournisseur sur les données clients.
- Sortie réseau vers le fournisseur IA uniquement, via un proxy en liste blanche.

### 3.10 Secrets (dépôt public)
- **Aucun secret dans le dépôt**, jamais : détection avant commit (gitleaks) + protection à l'envoi de GitHub (secret scanning push protection).
- Secrets à l'exécution via KMS ou OpenBao, identifiants à durée courte.
- Les fichiers de configuration publics ne contiennent que des valeurs d'exemple.

### 3.11 Exploitation
- Journaux structurés ; journal d'audit applicatif chaîné.
- **Supervision par le SOC de CyberAar (AarSOC)** : Sigma Books devient une référence client interne.
- Sauvegardes chiffrées, immuables, **restauration testée chaque mois** ; RPO et RTO définis.
- Plan de réponse aux incidents et procédure de notification des clients.

### 3.12 Transparence et divulgation
- `SECURITY.md` et `/.well-known/security.txt`.
- Signalement privé des vulnérabilités activé sur GitHub.
- **Programme de bug bounty sur AarMe** (la plateforme de CyberAar).
- Publication des avis de sécurité et des correctifs.
- Modèle de menaces et décisions d'architecture (ADR) publics dans le dépôt.

### 3.13 Conformité
- Loi sénégalaise n° 2008-12 sur les données personnelles : déclaration auprès de la CDP, registre des traitements, contrat de sous-traitance avec les cabinets.
- AUDCIF : conservation des pièces et des livres pendant 10 ans, numérotation chronologique, intangibilité après clôture.
- Certification ISO/IEC 27001 visée à moyen terme pour l'offre SaaS.

---

## 4. Licence et modèle open source

### 4.1 Licence proposée : AGPL-3.0-or-later
| Option | Avantage | Inconvénient |
|---|---|---|
| MIT / Apache-2.0 | Adoption maximale | Un concurrent peut reprendre le code et le vendre en SaaS sans rien rendre ; impossible d'intégrer du code GPL/AGPL |
| GPL-3 | Copyleft | Ne couvre pas l'usage SaaS : un hébergeur peut modifier sans publier |
| **AGPL-3** | Copyleft étendu au SaaS : quiconque héberge une version modifiée doit en publier le code | Certaines grandes entreprises évitent l'AGPL en interne |

**Ce que l'AGPL débloque :** on peut reprendre légalement (avec attribution) du code sous GPL-3, LGPL-3 et AGPL-3 — Tryton, Odoo Community, OCA, ERPNext, Dolibarr, Bigcapital, Axelor —, ainsi que tout le code sous licence permissive. Reste exclu le code « GPL-2 uniquement » (sans clause « ou ultérieure »), qui n'est pas compatible avec la GPL-3 ni l'AGPL-3.

### 4.2 La marque
Le code est libre, **le nom ne l'est pas**. « Sigma Books » et le logo sont déposés à l'**OAPI** (zone francophone d'Afrique), puis ailleurs si nécessaire. Une politique de marque publiée précise qu'un fork doit changer de nom. C'est ce qui protège Odoo et les autres éditeurs open source.

### 4.3 Contributions : CLA ou DCO
| Option | Effet |
|---|---|
| **CLA** (recommandé) | Les contributeurs accordent à la société une licence large sur leurs contributions. Garde la possibilité de vendre une **licence commerciale** (double licence) aux clients qui refusent l'AGPL. |
| DCO | Plus léger, plus apprécié des contributeurs, mais chaque contributeur garde ses droits : la double licence devient impossible. |

### 4.4 D'où viennent les revenus
| Source | Description |
|---|---|
| **Sigma Books Cloud** | SaaS hébergé, tenu à jour, sauvegardé et supervisé — c'est le cœur du chiffre d'affaires |
| **IA gérée** | Traitement des pièces facturé à l'usage (le coût de l'API IA est réel) |
| **Support et SLA** | Pour les cabinets et les entreprises qui s'auto-hébergent |
| **Licence commerciale** | Pour ceux qui ne veulent pas de l'AGPL (possible grâce au CLA) |
| **Formation et certification** | Cabinets et intégrateurs partenaires certifiés |

Point de vigilance : publier le code ne suffit pas à protéger l'activité. La valeur vendue est **le service** (hébergement sûr, IA, mises à jour réglementaires, support), plus la marque et l'écosystème de partenaires.

---

## 5. Modèle de déploiement

### 5.1 Ce que font les acteurs du secteur
| Éditeur | Code | Déploiement | Revenus |
|---|---|---|---|
| **Odoo** | *Open core* : Community (LGPL) + Enterprise (propriétaire) | SaaS (Odoo Online), PaaS (Odoo.sh), installation sur site | Abonnements Enterprise, hébergement, réseau de partenaires |
| **Frappe / ERPNext** | 100 % open source (GPL-3) | SaaS (Frappe Cloud), auto-hébergement | Hébergement, support, partenaires |
| **Dolibarr** | 100 % open source (GPL-3) | Auto-hébergement, hébergeurs tiers | Écosystème de prestataires |
| **Sage** | Propriétaire | Logiciel installé (Sage 100) en transition vers le cloud | Licences, abonnements, revendeurs |
| **Pennylane, Xero, QuickBooks Online** | Propriétaire | **SaaS uniquement** | Abonnements |

Tendance nette : le **SaaS multi-tenant** domine les nouveaux acteurs. L'installation sur site subsiste pour les grandes organisations et les secteurs sensibles.

### 5.2 Choix pour Sigma Books
| Offre | Pour qui | Quand |
|---|---|---|
| **Sigma Books Cloud** (SaaS multi-tenant) | Cabinets, PME | Dès le lancement |
| **Auto-hébergement communautaire** (binaire unique ou conteneur, sans support) | Développeurs, petites structures, évaluateurs | Dès la publication du code (l'AGPL le permet de toute façon) |
| **Instance dédiée gérée** (mono-tenant, hébergement au choix, y compris au Sénégal) | Grands cabinets, banques, administrations, exigences de souveraineté | Après les premiers clients Cloud |
| **Installation sur site avec support** | Organisations qui l'exigent | Sur demande, via des partenaires certifiés |

Le choix de Rust aide directement : un **binaire unique** et une base PostgreSQL suffisent pour installer Sigma Books, ce qui rend l'auto-hébergement et les instances dédiées simples à maintenir.

### 5.3 Souveraineté des données
L'hébergement de l'offre Cloud (centre de données au Sénégal ou fournisseur cloud international) reste à décider. Les critères : exigences de la CDP, latence, disponibilité, coût, et attentes des cabinets. L'instance dédiée permet de répondre aux clients qui exigent un hébergement local.

---

## 6. Briques open source réutilisables

### 6.1 Principe
- Le langage est Rust : le code Python, PHP ou Java ne se copie pas, il se **porte**.
- Avec Sigma Books sous AGPL-3, le portage de code GPL-3, LGPL-3 et AGPL-3 est **légal** (avec attribution). On peut donc suivre leur code au plus près et **reprendre leurs suites de tests comme jeux de données de référence**, ce qui est le plus précieux.

### 6.2 Références métier (à porter ou à étudier)
| Brique | Référence principale | Autres | Licence | Statut avec AGPL |
|---|---|---|---|---|
| Écritures, journaux, périodes, exercices | **Tryton** | Odoo, ERPNext | GPL-3+ | ✅ portable |
| Plan SYSCOHADA | **Tryton** (`account_syscohada`) | Odoo (modules pays OHADA) | GPL-3+ / LGPL-3 | ✅ — la source officielle reste le plan OHADA |
| Moteur de TVA | **Odoo** | Tryton | LGPL-3 | ✅ portable |
| Lettrage et règles de rapprochement | **Odoo + OCA** (`account-reconcile`) | ERPNext, Tryton | LGPL-3 / AGPL-3 | ✅ portable |
| Relevés bancaires (import, rapprochement) | **Tryton** (`account_statement`) | OCA (`bank-statement-import`), hledger (règles CSV) | GPL-3+ | ✅ portable |
| Immobilisations et amortissements | **Tryton** (`account_asset`) | OCA (`account_asset_management`), ERPNext, Axelor | GPL-3+ | ✅ portable |
| Clôture, report à nouveau | **Tryton** | ERPNext, Axelor | GPL-3+ | ✅ portable |
| États financiers paramétrables (poste → comptes) | **OCA `mis-builder`** | Odoo | AGPL-3 | ✅ portable |
| FEC (import, export) | **Tryton** (`account_fr`) | Axelor, Odoo | GPL-3+ | ✅ portable |
| Formats d'export vers d'autres logiciels | **Dolibarr** | — | GPL-3+ | ✅ portable |
| Inaltérabilité par hash | **Odoo** | — | LGPL-3 | ✅ portable |
| Architecture d'un logiciel comptable moderne | **Bigcapital** | Axelor | AGPL-3 | ✅ portable |
| Rapprochement pièces ↔ transactions avec IA | Midday | — | AGPL « usage non commercial » + licence commerciale | ⚠️ **étude seulement** (restriction commerciale) |
| — | Akaunting | — | BSL | ❌ à écarter |
| — | iDempiere, LedgerSMB, Beancount | — | GPL-2 | ⚠️ étude seulement, sauf si leur licence indique « ou ultérieure » (à vérifier fichier par fichier) |

### 6.3 Bibliothèques Rust utilisables directement (licences permissives, compatibles AGPL)
| Besoin | Bibliothèque | Remarque |
|---|---|---|
| Serveur HTTP | `axum` + `tokio` | Standard de fait |
| Accès PostgreSQL | `sqlx` | Requêtes SQL **vérifiées à la compilation** |
| Montants | `rust_decimal` | Décimal exact, jamais de flottant |
| TLS | `rustls` | TLS en Rust pur, sans OpenSSL |
| Autorisation | `cedar-policy` | Apache-2.0, maintenu par AWS |
| Passkeys | `webauthn-rs` | MPL-2.0, compatible |
| Hachage de mots de passe, cryptographie | RustCrypto (`argon2`, `sha2`, `aes-gcm`) | Implémentations auditées |
| CSV, Excel | `csv`, `calamine` (lecture), `rust_xlsxwriter` (écriture) | |
| Génération PDF des états financiers | **Typst** | Moteur de composition en Rust : modèles d'états SYSCOHADA sans navigateur sans tête |
| Observabilité | `tracing`, OpenTelemetry | |
| Tests | `proptest` (propriétés), `cargo-fuzz` (fuzzing), `insta` (snapshots) | |

### 6.4 Ce qui reste à construire nous-mêmes
Le pipeline IA de lecture des pièces, les états SYSCOHADA officiels complets, la fiscalité sénégalaise, le mobile money, l'export multi-logiciels et le portail client. Aucun logiciel open source ne les fournit : **c'est la valeur propre de Sigma Books**.

---

## 7. Organisation du dépôt GitHub

```
sigma-books/                      (organisation GitHub, dépôt principal public)
├── crates/
│   ├── compta-core/       ← moteur comptable PUR (aucune I/O) : types, invariants,
│   │                         enregistrement, lettrage, TVA, amortissements, clôture, états
│   ├── syscohada/         ← plan de comptes, troncs, correspondance postes ↔ comptes
│   ├── fiscal-sn/         ← paramètres fiscaux Sénégal (versionnés, sourcés)
│   ├── formats/           ← FEC, profils d'import/export, relevés (fuzzés en continu)
│   ├── integrite/         ← chaîne de hachage, horodatage, outil de vérification
│   ├── ia/                ← orchestration IA (propositions uniquement)
│   ├── authz/             ← politiques Cedar + leurs tests
│   ├── api/               ← serveur axum
│   └── worker-documents/  ← traitement isolé des documents
├── web/                   ← interface SvelteKit (TypeScript strict)
├── site/                  ← site public et documentation (Astro)
├── docs/
│   ├── adr/               ← décisions d'architecture
│   ├── menaces/           ← modèle de menaces
│   └── provenance/        ← code porté : source, licence, attribution
├── SECURITY.md · LICENSE (AGPL-3.0) · TRADEMARK.md · CONTRIBUTING.md (CLA)
└── .github/               ← CI, CODEOWNERS, politiques de branche
```

Règles :
- `compta-core` ne dépend d'aucune base, d'aucun réseau, d'aucune horloge système. Il est testé par propriétés (par exemple : « pour toute suite d'écritures valides, la balance est équilibrée »).
- Les **données d'évaluation de l'IA** issues de vrais clients ne sont **jamais** dans le dépôt public. Seuls des jeux synthétiques ou anonymisés avec accord y figurent.
- Tout code porté depuis un autre projet est documenté dans `docs/provenance/` (projet, fichier, licence, auteurs).

---

## 8. Cycle de développement sécurisé

| Étape | Contrôles |
|---|---|
| Conception | Modèle de menaces mis à jour pour chaque fonctionnalité ; ADR pour chaque décision structurante |
| Code | `clippy` en mode strict (avertissements bloquants), `#![forbid(unsafe_code)]` dans toutes les crates métier, Semgrep |
| Dépendances | `cargo-deny` (vulnérabilités, licences, sources), `cargo-vet` (audit des dépendances), politique de dépendances minimales, Renovate |
| Revue | Branches protégées, commits signés, **deux relecteurs** sur `compta-core`, `integrite`, `authz` et `api` (CODEOWNERS) |
| Tests | Unitaires, propriétés, fuzzing des analyseurs, tests d'isolation entre tenants à chaque build, tests de politiques Cedar |
| Build | GitHub Actions épinglées par hash ; builds reproductibles ; **SBOM** (CycloneDX) ; signature des artefacts (Sigstore) ; provenance SLSA |
| Livraison | Images minimales (distroless), sans shell ; scan des images ; déploiement progressif |
| Exploitation | DAST (OWASP ZAP) sur la préproduction, pentest externe avant chaque version majeure, bug bounty AarMe en continu |

---

## 9. Feuille de route

La v1 est le logiciel comptable complet. Le plan de développement détaillé (lots L0 à L9, environ 30 à 34 semaines avec parallélisation) est dans la **Spécification fonctionnelle v1.0, §24**. Les Fondations correspondent au lot **L0** ; les exigences de sécurité s'appliquent à tous les lots.

---

## 10. Décisions validées

| Sujet | Décision |
|---|---|
| Langage | **Rust** (backend et moteur) ; TypeScript strict + SvelteKit (interface) ; Astro (site public) |
| Contributions | **CLA** standard (modèle inspiré d'Apache), géré par le bot CLA Assistant |
| Hébergement de l'offre Cloud | **Grand fournisseur cloud, région Europe**, au lancement. Transfert hors du Sénégal couvert par la déclaration CDP avant le premier client. Instances dédiées hébergées au Sénégal proposées ensuite. |
| Marque | **Recherche d'antériorité** sur « Sigma Books », puis dépôt **OAPI** |
| IA | Prompts **publics** ; données d'évaluation réelles **toujours privées** |
| Publication du code | **Public dès le premier commit** |
| Périmètre de la v1 | **Logiciel comptable complet** (Spécification fonctionnelle v1.0) |

*Sigma Books — Fondations techniques — Novembre 2026*
