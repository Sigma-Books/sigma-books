# 0010 — Horodatage RFC 3161 de la tête de chaîne

- **Statut :** proposée
- **Date :** 2026-11-18

## Contexte
La chaîne de hachage (ADR 0007) détecte toute altération d'un export, sauf une réécriture complète par celui qui maîtrise tout l'export : il peut recalculer la chaîne entière. Seule une **référence externe** datée le détecte. La spec (§6.4) prévoit un horodatage RFC 3161 de la tête de chaîne « chaque jour et à chaque clôture ».

Le mainteneur a délégué le choix de l'autorité et de la fréquence (question Q5 du lot L1, 2026-11-18). Aucun moteur comptable de référence n'utilise d'horodatage externe : Odoo chaîne ses écritures et laisse l'utilisateur conserver un rapport d'intégrité ; Dolibarr archive son journal inaltérable par mois, et sa certification par une autorité distante est désactivée ; Tryton, Bigcapital et OCA n'ont pas de chaîne.

## Décision
- **Objet horodaté : le témoin** `sigma-books temoin v1` (identifiant du dossier, empreinte de tête, nombre d'écritures ; `docs/integrite/format-v2.md` §4). On soumet son empreinte SHA-256 : rien d'autre ne quitte l'instance, aucune donnée comptable.
- **Autorités** : liste configurable, essayée dans l'ordre. Par défaut **DigiCert** (`http://timestamp.digicert.com`), puis **Sectigo** (`http://timestamp.sectigo.com`) en secours. Deux autorités publiques, gratuites, adossées à des racines largement reconnues, sans compte ni clé d'API. Leurs racines sont intégrées au vérificateur (`crates/integrite/src/horodatage/ancres/`) ; `sigma-verifier --ancre` en ajoute d'autres.
- **Fréquence** (`horodatage_requis`) :
  - chaque jour, si la chaîne a changé depuis le dernier jeton (au plus un jeton par jour) ;
  - à chaque verrouillage de période et à chaque clôture d'exercice, le jour même.
  - **Rejeu différé** : la règle ne regarde que le dernier jeton obtenu. Une autorité injoignable laisse la règle vraie ; la demande est rejouée au passage suivant, sur la tête du moment, qui couvre aussi les écritures restées sans jeton.
- **Vérification hors ligne** (`verifier_jeton`) : empreinte du témoin, attributs signés CMS, certificat désigné par `signingCertificate(V2)`, usage étendu unique et critique `timeStamping`, signature RSA PKCS#1 v1.5 (SHA-256, 384, 512), chaîne jusqu'à une ancre, chaque certificat valide **à la date du jeton**.
- **Export v2** : les jetons accompagnent l'export (`format-v2.md`). `sigma-verifier` rattache chaque jeton à l'état de la chaîne qu'il décrit et indique, pour chaque jeton, les écritures couvertes et la date. Le format canonique des écritures et la chaîne sont inchangés ; l'export v1 reste lisible.
- **Dépendances** (RustCrypto) : `x509-tsp`, `cms`, `x509-cert`, `der`, `rsa`, `sha1`, `base64ct`. `x509-tsp` et `cms` n'existent qu'en **préversion** (`0.2.0-rc.0`, `0.3.0-rc.2`) pour la lignée `der 0.8` déjà utilisée par `x509-cert 0.3` : versions épinglées exactement (`=`), à remplacer par les versions stables dès leur publication.

## Conséquences
- Une réécriture complète de l'export est détectée dès qu'un jeton couvre l'écriture modifiée : `sigma-verifier` signale le témoin qui ne correspond plus. Les écritures postérieures au dernier jeton restent couvertes seulement par la chaîne (fenêtre d'un jour au plus en régime normal).
- Le worker (lot L2) envoie les requêtes HTTP ; le cœur reste pur (nonce et date passés en paramètre).
- Tests sur des jetons réels de DigiCert et de Sectigo, figés dans `crates/integrite/tests/vecteurs/horodatage/` ; la requête produite est identique, octet pour octet, à celle d'`openssl ts -query`.
- Limites connues : la **révocation** des certificats de l'autorité n'est pas contrôlée (vérification hors ligne) ; seules les signatures RSA sont prises en charge (celles des deux autorités retenues).
- **Point à soumettre à l'expert-comptable et à un juriste** : valeur probante, dans l'espace OHADA et au Sénégal, d'un horodatage par une autorité étrangère non qualifiée localement ; existence éventuelle d'un prestataire national reconnu, qui s'ajouterait à la liste des autorités.

## Alternatives écartées
- **FreeTSA** en secours : service bénévole, sans engagement de disponibilité ; Sectigo est préféré.
- **Une autorité unique** : un incident chez elle suspendrait l'horodatage.
- **Horodatage après chaque lot de validations** (au plus une fois par heure) : plus de requêtes pour un gain faible, la validation se faisant au plus par période d'un mois (AUDCIF art. 22 2°).
- **Implémentation maison de l'ASN.1 ou de RSA** : contraire à la règle « RustCrypto uniquement » du crate `integrite`.
- **OpenSSL** (liaison C) : contraire à la règle de sécurité mémoire (ADR 0001) ; OpenSSL sert seulement de contre-vérification hors dépôt.
