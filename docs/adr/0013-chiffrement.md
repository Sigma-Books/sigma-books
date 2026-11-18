# 0013 — Chiffrement : disques obligatoires, chiffrement par cabinet facultatif

- **Statut :** acceptée (2026-11-18, par le mainteneur)
- **Date :** 2026-11-18

## Contexte
Les Fondations (§3.7) prévoient une clé de données par cabinet (chiffrement d'enveloppe) et la suppression cryptographique à la résiliation. Le lot L2 proposait de chiffrer dans l'application les pièces et les textes libres, avec OpenBao (`docs/lots/L2.md`, question 2, option a).

Ce chiffrement a un revers. Une clé de cabinet perdue rend illisibles dix ans d'archives légales (F3.6). Or le droit exige de les restituer :
- AUDCIF art. 24 (p. PDF 24) : livres et pièces justificatives conservés **dix ans** ;
- AUDCIF art. 22 7° (p. PDF 24) : support assurant « la conservation et la restitution en clair » du contenu « pendant les délais requis » ;
- CGI Sénégal art. 637 I et II (p. PDF 306) : dix ans, y compris sur support informatique.

Le mainteneur a tranché le 2026-11-18 (question 2) : ne pas prendre le risque de perdre des données financières. L'effort porte sur un **cloisonnement parfait** (spec §2 principe 12 ; Cedar, requête et RLS, ADR 0005 et 0011), pas sur des clés par cabinet.

Deux niveaux sont à distinguer :
1. le **chiffrement des disques** des serveurs, contre le vol d'un support ;
2. le **chiffrement applicatif par cabinet**, contre un lecteur qui accède à la base ou au stockage.

Menaces visées : F2.8 (vol de sauvegarde ou de disque), F3.3 (effacement à la résiliation), F3.6 (perte d'une clé), F9.2 (opérateur lisant les données d'un cabinet).

Moteurs de référence : **aucun** ne chiffre par société (Tryton, Odoo, OCA, Dolibarr, Bigcapital). **Dolibarr** chiffre certains champs en AES-256-CTR, **sans authentification** (`htdocs/blockedlog/lib/securitycore.lib.php:28`), et **renvoie la valeur en clair** si la clé manque (même fichier, l. 96 et 119).

## Décision
### 1. Chiffrement des disques : obligatoire
- Tout volume qui porte des données de cabinet est chiffré : base PostgreSQL (données et WAL), stockage objet des pièces (ADR 0015), sauvegardes, volumes temporaires des exports.
- Moyens admis : LUKS/dm-crypt, BitLocker, chiffrement des volumes ou des objets par l'hébergeur, ou équivalent.
- Les clés sont gérées par l'exploitation, hors du serveur applicatif quand le moyen le permet. Elles sont séquestrées séparément des données et des sauvegardes.
- Sigma Books ne peut pas le constater lui-même : c'est une **exigence de déploiement**. Elle figure dans la documentation d'exploitation et dans la liste de contrôle de mise en production, vérifiée en **L9** (avec la restauration testée, F9.3).
- Le transport reste en TLS 1.3 (Fondations §3.7).

### 2. Chiffrement applicatif par cabinet : facultatif, désactivé par défaut
- **Non développé en L2.** Aucune donnée n'est chiffrée par l'application avec une clé de cabinet.
- S'il est développé plus tard, c'est une **option choisie par le client**, jamais imposée. Avant l'activation, l'administrateur du cabinet reçoit un avertissement écrit : la perte de la clé rend illisibles dix ans d'archives, et ni l'éditeur ni l'exploitant ne pourront les restituer.
- Un nouvel ADR fixera alors les champs chiffrés, le fournisseur de clés, le détenteur de la clé, la rotation et la sauvegarde des clés. Il respectera les principes suivants :
  - **chiffrement authentifié** (AEAD, par exemple AES-256-GCM, RustCrypto) ; données associées : identifiants du cabinet, du dossier, de la colonne et de la ligne, pour qu'un texte chiffré déplacé ailleurs soit refusé ;
  - **chiffrement d'enveloppe** : clé de données par cabinet, protégée par une clé maîtresse dans un fournisseur derrière une interface (OpenBao Transit ou KMS) ; fournisseur local refusé en production ;
  - **identifiant de clé** (et version d'algorithme) en tête de chaque texte chiffré, pour la rotation et la lecture des anciennes versions ;
  - **refus de démarrer**, ou de servir le cabinet, sans sa clé ; jamais d'écriture en clair en repli ;
  - recherche sur un champ chiffré par **empreinte HMAC-SHA-256** à clé propre au cabinet, en égalité seulement ;
  - montants, comptes, dates et identifiants restent en clair (règle absolue 2, RLS) ;
  - le format canonique et la chaîne de hachage (ADR 0007) portent sur le contenu en clair ; `sigma-verifier` est inchangé ;
  - clés sauvegardées, restauration testée **clés comprises** (F3.6), accès aux clés journalisé (F9.2).

### 3. Résiliation d'un cabinet
1. **Remise des archives** au client : export complet (écritures, pièces, journal d'audit, export d'intégrité, jetons d'horodatage) et `sigma-verifier`. La conservation de dix ans lui incombe ensuite.
2. **Effacement réel** des lignes en base et des objets du stockage, par une procédure d'exploitation à double validation. Le rôle applicatif n'a ni `DELETE` sur les écritures validées (ADR 0004) ni droit de lever le verrou WORM (ADR 0015) : la procédure passe par des rôles d'exploitation dédiés et journalisés.
3. **Expiration des sauvegardes** au terme de leur durée de rétention, fixée en L9 et annoncée au contrat.

Pas de suppression cryptographique.

### Hors du champ de cet ADR
Les secrets d'authentification (secret TOTP) sont chiffrés par une clé de l'instance (ADR 0012) : sa perte n'efface aucune donnée comptable.

## Conséquences
- **F2.8** est couvert par le chiffrement des disques et des sauvegardes. Un vol logique (accès SQL avec des identifiants) relève des rôles et de la RLS (ADR 0004, 0011), pas du chiffrement.
- **F9.2** : risque **accepté par défaut**. Un opérateur qui accède à la base ou au stockage lit les données. Il est réduit par des accès d'exploitation nominatifs, à durée courte et journalisés (F9.1), la supervision par le SOC, et des rôles de base distincts.
- **F3.6** : sans objet tant que l'option n'est pas activée. Le risque se déplace sur les clés de disque et de sauvegarde : séquestre par l'exploitation et restauration mensuelle testée (L9).
- **F3.3** : remplacé par l'effacement réel ; la procédure de résiliation est testée en L9.
- **F3.2** repose sur le cloisonnement seul : Cedar, requête, RLS, clés d'objet non devinables, aucune URL publique (ADR 0015).
- Le lot L2 perd un composant (fournisseur de clés). La PR 10 et l'issue L2-21 se réduisent à l'exigence de déploiement ; les tests « Chiffrement » de `docs/lots/L2.md` sont sans objet en L2.
- Fondations §3.3 et §3.7, spec §24 (L2) et modèle de menaces v0.3 (F2.8, F3.2, F3.3, F3.6, F9.2, F9.3) suivent cette décision. **Reste à aligner** : `docs/lots/L2.md` (objectif, choix « Chiffrement », question 1a « chiffré par cabinet », PR 10, tests).

## Alternatives écartées
- **Chiffrement applicatif des pièces et des textes libres, actif par défaut, avec OpenBao** (question 2a) : risque de perte des archives pour tous les cabinets ; fournisseur de clés, rotation et séquestre à exploiter dès L2 ; gain limité à la menace de l'opérateur.
- **Tout chiffrer** (question 2b) : contraire à `NUMERIC` (règle absolue 2) et à la RLS.
- **Pièces seulement** (question 2c) : même risque de perte, sur les pièces, qui sont précisément la preuve à conserver.
- **Suppression cryptographique** à la résiliation : suppose le chiffrement applicatif ; effacement réel et expiration des sauvegardes suffisent.
- **Chiffrement sans authentification ou repli en clair** (Dolibarr) : altération indétectable ; donnée exposée sans alerte.
- **Chiffrement des disques seulement recommandé** : un support volé exposerait tous les cabinets à la fois.
- **Chiffrement transparent de PostgreSQL (TDE)** : absent de PostgreSQL communautaire ; le chiffrement du volume donne la même protection contre le vol de support.
