# 0015 — Pièces justificatives en L2 : stockage objet WORM, sans analyse ni rendu

- **Statut :** acceptée (2026-11-18, par le mainteneur)
- **Date :** 2026-11-18

## Contexte
Toute écriture validée référence au moins une pièce (spec §2 principe 2 ; AUDCIF art. 17 et 22 7°). Depuis #104, elle la référence par son empreinte SHA-256 (invariant 6). Le stockage des pièces était prévu en L4, avec l'antivirus, la rasterisation et le bac à sable (Fondations §3.8). Sans stockage en L2, aucun vrai dossier ne peut être tenu, et aucun auditeur ne peut rapprocher une empreinte d'un fichier.

Le mainteneur a tranché le 2026-11-18 : question 1, option a (pièces internes et dépôt de pièces externes stockées telles quelles) ; question 12, option c (stockage objet compatible S3, neutre sur l'hébergeur).

Exigences :
- conservation **dix ans** des pièces (AUDCIF art. 24, p. PDF 24 ; CGI Sénégal art. 637, p. PDF 306) ;
- pièces stockées **par dossier**, immuables, relation N-N avec les écritures, rôle principale ou complémentaire (spec §6.5) ;
- cloisonnement : doublons cherchés **dans le dossier seulement** (spec §2 principe 12, §9.3 ; modèle de menaces F3.5 et Q12).

Menaces visées : F3.1 (remplacement d'une pièce après comptabilisation), F3.2 (lecture des pièces d'un autre cabinet), F3.5 (déduplication comme canal de fuite), F3.4 (saturation, part L2), F1.5 (contenu actif servi par l'application).

Moteurs de référence : **aucun** ne rend une pièce intouchable après validation. **Odoo** lit chaque PDF ou XML joint à une facture (`addons/account/models/ir_attachment.py`). **Dolibarr** affiche les fichiers HTML déposés. **Bigcapital** stocke sur S3 en **lecture publique** (`modules/Attachments/Attachment.module.ts:89`), sans limite de taille, mais il a un lien N-N entre pièces et documents. **Tryton** nomme ses fichiers par un UUID aléatoire (`trytond/filestore.py`, `_id`).

## Décision
### Deux sortes de pièces
- **Pièce interne** : saisie dans Sigma Books (en L2, note d'opérations diverses datée et motivée, spec §2 principe 2). Elle est sérialisée sous une forme canonique documentée dans la PR 12, hachée, puis stockée comme une pièce externe. Une correction crée une nouvelle pièce.
- **Pièce externe** : fichier déposé, **stocké tel quel**.

### Dépôt d'une pièce externe
- **Types admis par liste blanche**, reconnus par leur **signature** (premiers octets), jamais par l'extension ni par le type annoncé : PDF, JPEG, PNG en L2. Tout autre type est refusé. Le type retenu est celui de la signature. *(Amendé par l'ADR 0034, 2026-11-27 : un type « texte » — relevé bancaire CSV — s'ajoute pour le lot L3. Faute de signature, il est reconnu par son **contenu** (aucun octet nul, aucun caractère de contrôle hors tabulation et fins de ligne, encodage d'une liste fermée, longueur de ligne bornée), admis **seulement par la route de dépôt de relevé**, limité à 5 Mio (valeur initiale, appliquée pendant la réception), jamais rendu et lu seulement par l'analyseur pur de `formats`. Les types à signature priment ; le reste de cet ADR est inchangé.)*
- **Limite de taille** : 20 Mio par fichier par défaut, configurable. Elle est appliquée pendant la réception du flux, avant tout stockage (F1.9).
- **Empreinte SHA-256 calculée par le serveur** sur les octets reçus. Une empreinte fournie par le navigateur, si elle diffère, fait refuser le dépôt.
- **Aucune analyse, aucun rendu** côté serveur en L2 : ni lecture du PDF, ni miniature, ni extraction de métadonnées. Seuls les premiers octets sont lus pour la signature. Antivirus, rasterisation et bac à sable : L4 (Fondations §3.8).
- **Doublons** : une empreinte déjà présente **dans le même dossier** donne un avertissement, sans blocage ; l'ajout confirmé est journalisé (spec §9.3). Aucune comparaison avec un autre dossier ni un autre cabinet ; aucune réponse ne dépend de leur contenu (F3.5).
- Le nom de fichier d'origine est une métadonnée. Il ne sert jamais de chemin de stockage.

### Stockage
- **Stockage objet compatible S3**, configurable (point d'accès, compartiment, région ; identifiants par les secrets d'exécution). Aucune dépendance à un hébergeur particulier ; l'instance peut viser un service local, un hébergeur au Sénégal ou un grand fournisseur.
- **WORM** : versionnement et **verrouillage d'objet** obligatoires. Rétention posée à l'écriture : **dix ans** au moins. Le verrou peut seulement être prolongé.
- Mode de verrouillage « gouvernance » : le rôle applicatif ne peut ni remplacer, ni supprimer, ni lever le verrou. Seule la procédure de résiliation de l'exploitation, à double validation, peut effacer (ADR 0013 ; modèle de menaces Q14).
- Au démarrage en production, le serveur **refuse de démarrer** si le compartiment n'a pas le versionnement et le verrouillage d'objet.
- **Clé d'objet non devinable** : `<cabinet>/<dossier>/<UUID v4>`, sans l'empreinte ni le nom de fichier.
- Le chiffrement repose sur celui des disques ou des objets de l'hébergeur (ADR 0013) ; pas de chiffrement applicatif.
- Ordre d'écriture : objet d'abord, puis ligne en base dans la transaction de la requête. Un objet sans ligne n'est jamais servi ; une tâche de la file le signale.

### Lien entre pièces et écritures
- Relation **N-N**, avec le rôle de la pièce (principale ou complémentaire, spec §6.5).
- Au brouillard, le lien se crée et se retire librement ; chaque changement est journalisé.
- Dès qu'**une écriture liée est validée**, son lien est **verrouillé** : il entre dans le contenu canonique haché (invariant 6), et la pièce ne peut plus être retirée.
- Une pièce jamais liée à une écriture validée peut être **écartée** (dépôt erroné) : elle disparaît des listes, l'action est journalisée, l'objet reste sous verrou jusqu'au terme de sa rétention.

### Accès et téléchargement
- Dépôt et consultation selon les droits du **dossier** (spec §20, « Déposer des pièces » ; politiques Cedar, ADR 0005), toujours par l'API après décision Cedar, contexte de tenant et RLS. Pièce d'autrui et pièce absente donnent la même réponse (F1.8).
- Aucune URL publique ni présignée durable (F3.2).
- Téléchargement **toujours en pièce jointe** : `Content-Disposition: attachment` (nom encodé selon RFC 6266), `X-Content-Type-Options: nosniff`, `Content-Security-Policy: sandbox; default-src 'none'`. Pas d'aperçu dans l'interface en L2.
- L'empreinte est **recalculée à la lecture** ; un écart bloque le téléchargement et lève une alerte de sécurité (F3.1).

## Conséquences
- F3.1 et F3.2 sont avancées de L4 à L2 pour le stockage ; F3.5 est traitée par la recherche de doublons au sein du dossier.
- Un composant de plus : un stockage objet avec verrouillage d'objet, en développement, en CI et en production. Le service de développement et le client S3 (crate Rust) sont choisis et justifiés dans la PR 12.
- Aucun analyseur de format : pas de cible de fuzzing en L2. *(Note du 2026-11-27 : L3 introduit l'analyseur de relevés CSV de `crates/formats`, avec sa cible de fuzzing obligatoire ; ADR 0034.)* Les tests portent sur la signature, la limite de taille, le refus de réécriture, l'empreinte à la lecture, l'isolation du téléchargement et l'absence de doublon signalé hors du dossier.
- Les quotas par cabinet et par dossier (F3.4) restent en L4 ; L2 n'a que la limite par fichier.
- Une pièce déposée par erreur reste stockée jusqu'au terme de sa rétention. Le registre des traitements (modèle de menaces §8) devra le mentionner.
- **Point à valider** (spec §26 ; expert-comptable et fiscaliste) : **CGI Sénégal art. 640** (p. PDF 311). Les factures émises et reçues sont stockées **sur le territoire sénégalais** ; stockées par voie électronique, elles restent accessibles **en ligne** à l'administration fiscale. La décision ministérielle d'application n'est pas disponible. Le stockage configurable permet un compartiment au Sénégal ; un accès de l'administration serait un nouveau rôle, à concevoir dans un autre ADR. Ceci touche aussi le choix de l'hébergeur (Fondations §5.3).
- **Point à valider** : point de départ de la rétention (AUDCIF art. 24 ne le fixe pas ; CGI art. 637 : dernière opération des livres ou date de la pièce). Dix ans depuis le dépôt est un minimum ; le prolongement jusqu'à la clôture de l'exercice plus dix ans est à confirmer.

## Alternatives écartées
- **Empreinte calculée dans le navigateur, fichier gardé par le cabinet** (question 1b) : le serveur ne pourrait rien recalculer, l'auditeur rien vérifier.
- **Pièces internes seulement** (question 1c) : pas de vrai dossier en L2.
- **Fichiers en base** (`bytea`, objets larges) : base et sauvegardes alourdies, pas de verrou WORM.
- **Système de fichiers local** : pas de WORM, une seule machine.
- **Stockage unique par empreinte entre dossiers ou cabinets** : canal de fuite (F3.5), contraire au principe 12.
- **Lecture ou rendu côté serveur** (Odoo, Dolibarr) : exécute du code d'analyse sur un fichier hostile hors bac à sable ; reporté en L4, dans le worker isolé.
- **Accès public ou URL présignée durable, sans limite de taille** (Bigcapital) : fuite et saturation.
- **Pièce modifiable après validation** (tous les moteurs étudiés) : contraire à F3.1 et au caractère définitif (ADR 0007).
- **Mode de verrouillage « conformité »** : interdit tout effacement, même à la résiliation après remise des archives (ADR 0013).
