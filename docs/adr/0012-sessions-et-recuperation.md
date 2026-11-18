# 0012 — Sessions, secours, récupération de compte et création des cabinets

- **Statut :** proposée
- **Date :** 2026-11-18

## Contexte
L'ADR 0006 pose les passkeys en principal, le TOTP en secours, l'absence de SMS, les attributs du cookie et la ré-authentification. Il laisse ouverts quatre points, que le lot L2 doit trancher avant d'écrire l'authentification (`docs/lots/L2.md`, PR 8 et 9) :
- le facteur qui accompagne le TOTP (la spec §21 dit « passkeys ou TOTP obligatoires ») ;
- la durée et la révocation des sessions ;
- la récupération d'un compte qui a perdu ses facteurs, maillon faible habituel (F1.12) ;
- la création des cabinets et l'envoi des invitations.

Le mainteneur a répondu le 2026-11-18 aux questions 3 (option b), 4 (option a) et 8 (option a) du lot L2.

Menaces visées : F1.1 (hameçonnage), F1.2 (vol ou rejeu de session), F1.4 (CSRF), F1.5 (XSS : le cookie reste hors de portée du script), F1.12 (récupération détournée), F12.1 et F12.2 (courriels).

Moteurs de référence (`references/moteurs_compta/`, Fondations §6.2) :
- **Dolibarr** fait passkey, mot de passe et TOTP ; la passkey exige la vérification de l'utilisateur (`'required'`, `htdocs/core/ajax/passkeyregister.php:109`). Mais son cookie est en `SameSite=Lax`, et il envoie le mot de passe en clair par courriel lors d'une réinitialisation.
- **Bigcapital** hache ses codes de récupération et refuse le rejeu d'un code TOTP (`modules/Auth/two-factor/`). Mais ses sessions sont des JWT non révocables, et ses jetons de réinitialisation sont stockés en clair (`modules/Auth/models/PasswordReset.ts`).
- **Tryton** refuse les mots de passe d'une liste interdite (`res/user.py:283-289`), hache en Argon2 et distingue la session « fraîche » des actions sensibles (`fresh_session`). Il envoie lui aussi un mot de passe par courriel.

Cet ADR **complète** l'ADR 0006 ; il ne le remplace pas.

## Décision
### Facteurs
- **Passkey (WebAuthn) en principal**, par `webauthn-rs`. Vérification de l'utilisateur (`userVerification: required`) à l'enrôlement et à chaque connexion. Un compte n'est actif qu'après l'enrôlement d'une passkey (MFA dès la création, ADR 0006). L'interface invite à enregistrer une seconde passkey.
- **Secours : mot de passe + TOTP, toujours ensemble**, activé au choix de l'utilisateur.
  - Mot de passe : Argon2id (crate `argon2`, RustCrypto), au moins 19 Mio de mémoire et 2 itérations (repère OWASP). Douze caractères au minimum, aucune règle de composition (NIST SP 800-63B). Refus de tout mot de passe présent dans une **liste de mots de passe compromis embarquée** dans le binaire, à la création comme au changement.
  - TOTP : RFC 6238, 6 chiffres, pas de 30 s, tolérance d'un pas. **Anti-rejeu** : le dernier pas accepté est mémorisé ; un code déjà utilisé est refusé. Le secret TOTP est chiffré (AEAD) par une clé de l'instance, tenue hors de la base ; sa perte n'oblige qu'à réenrôler le TOTP.
- **Codes de récupération** : dix codes à usage unique, d'au moins 80 bits d'aléa chacun, remis une seule fois à l'enrôlement. Stockés **hachés** (SHA-256 : leur aléa rend un dictionnaire inutile). Une nouvelle série annule l'ancienne. Le nombre restant est affiché.
- Aucun SMS, aucun lien de connexion par courriel (ADR 0006).

### Sessions
- Jeton **opaque** de 256 bits, tiré par `api` (l'aléa reste hors de `compta-core`). Stocké **haché** (SHA-256) en base ; jamais écrit dans les journaux (F2.11).
- Cookie `__Host-sigma_session` : `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, sans `Domain`.
- **Durées** : 30 minutes d'inactivité, 12 heures au plus depuis la connexion. La configuration peut seulement les raccourcir.
- **Rotation** du jeton à la connexion, à chaque ré-authentification et au changement de cabinet actif (utilisateur membre de plusieurs cabinets, F1.14).
- **Ré-authentification** : une action sensible exige une vérification de facteur (passkey, ou mot de passe + TOTP) datant de **moins de 5 minutes** (session « fraîche »). Actions concernées : celles de l'ADR 0006 et de la spec §21 (validation par lot, clôture, déverrouillage, export complet ou d'intégrité, droits), plus la gestion de ses propres facteurs et la désactivation de la séparation des tâches.
- **Anti-CSRF** : jeton lié à la session, exigé en en-tête sur toute mutation, en plus de `SameSite=Strict`.
- **Révocation immédiate** : chaque requête relit la session en base. L'utilisateur et l'administrateur du cabinet voient les sessions actives et les révoquent (spec §21). Un changement de droits ou de facteurs révoque les autres sessions de l'utilisateur.

### Récupération de compte (F1.12)
Ordre proposé à l'utilisateur : autre passkey, puis secours (mot de passe + TOTP), puis code de récupération, puis réinitialisation par un administrateur.
- **Un code de récupération** ouvre une session restreinte : elle permet seulement d'enrôler une nouvelle passkey, sans accès aux dossiers.
- **Réinitialisation par un administrateur du cabinet** :
  1. l'administrateur, ré-authentifié, déclare une **vérification hors bande** (appel ou rencontre) et son motif ;
  2. **délai de 24 heures** : l'utilisateur et les autres administrateurs sont notifiés aussitôt et peuvent annuler ;
  3. au terme du délai, un lien d'enrôlement à usage unique, valable **1 heure**, est envoyé ; il ne permet que d'enrôler une passkey ;
  4. les sessions, passkeys, secours et codes de l'utilisateur sont révoqués.
- Un administrateur ne réinitialise jamais son propre compte. Pour un administrateur sans autre administrateur dans le cabinet : procédure de support de l'instance, par commande d'administration, avec vérification d'identité, même délai et mêmes notifications.
- Jamais de récupération par courriel seul, jamais de mot de passe dans un courriel.
- Chaque étape est inscrite au journal d'audit (ADR 0014) et alertée.

### Jetons d'invitation et de réinitialisation
256 bits d'aléa, à usage unique, stockés **hachés** (SHA-256), jamais en clair. Validité : **24 heures** pour une invitation, **1 heure** pour un lien de réinitialisation. Le lien pointe vers le seul domaine de l'instance (F12.1).

### Création des cabinets (question 4a)
- Une **commande d'administration de l'instance** (sous-commande du binaire `api`, exécutée sur le serveur par l'exploitation) crée un cabinet et invite son premier administrateur. Aucune route HTTP ne crée de cabinet ; pas d'inscription libre en L2.
- L'administrateur du cabinet invite ensuite les autres utilisateurs, avec leur rôle. Le compte n'existe qu'après l'enrôlement d'une passkey.
- Un utilisateur existant invité dans un second cabinet garde son compte : l'appartenance s'ajoute après acceptation, en session authentifiée.

### Courriels (question 8a)
- **Relais SMTP configurable** (hôte, port, TLS obligatoire) ; identifiants fournis par les secrets d'exécution, jamais dans le dépôt.
- Envoi par la file de tâches (ADR 0004), hors de la transaction de la requête.
- Modèles constants et sobres, **sans montant, sans pièce ni nom de tiers** (F12.2) : invitation, réinitialisation, alertes de sécurité (nouvelle connexion, connexion par secours, code de récupération utilisé, réinitialisation demandée, changement de droits, export complet).
- SPF, DKIM et DMARC `p=reject` du domaine d'envoi relèvent de l'exploitation (F12.1).

## Conséquences
- F1.1 : aucune connexion sans facteur fort ; le mot de passe seul n'ouvre rien.
- F1.2, F1.4, F1.5 : cookie inaccessible au script, tournant et révocable ; mutations protégées deux fois.
- F1.12 : la récupération ne contourne jamais le facteur fort ; une réinitialisation se voit 24 heures avant d'aboutir.
- Le risque d'un administrateur malveillant qui réinitialise un compte (F1.15) reste ; il est réduit par le délai, les notifications aux autres administrateurs et l'audit (question 10 du lot).
- Coût : une lecture indexée de la session par requête. La liste de mots de passe compromis grossit le binaire : sa taille et son format sont choisis dans la PR 8.
- Dépendances à justifier dans la PR 8 : `webauthn-rs` (MPL-2.0), `argon2` ; implémentation du TOTP (crate dédiée, ou `hmac` + `sha1` de RustCrypto).
- Tests exigés : ceux de `docs/lots/L2.md` (« Authentification et sessions »), plus l'anti-rejeu TOTP, l'expiration des jetons et l'annulation d'une réinitialisation pendant le délai.
- Le portail client (L4, F1.3) suivra les mêmes règles, avec une durée de session plus courte.

## Alternatives écartées
- **Passkey seule** (question 3a) : un utilisateur sans seconde passkey ni code dépendrait d'un administrateur pour tout incident d'appareil.
- **Mot de passe + TOTP en méthode principale** (question 3c) : contraire à l'ADR 0006 ; il faudrait un ADR qui le remplace.
- **Jetons JWT sans état** (Bigcapital) : impossibles à révoquer avant expiration, contraire à la spec §21.
- **`SameSite=Lax`** (Dolibarr) : laisse passer les navigations de premier niveau venues d'un autre site.
- **Mot de passe envoyé par courriel** (Tryton, Dolibarr) et **récupération par courriel seul** : le courriel devient le vrai facteur.
- **Jetons stockés en clair** (Bigcapital) : une lecture de la base suffirait à prendre un compte.
- **SMS** : écarté par l'ADR 0006 (SIM swapping).
- **Inscription libre** (question 4b) : suppose facturation, quotas et anti-abus, absents de L2.
- **Notifications dans l'application seulement** (question 8b) : le lien d'invitation circulerait par des canaux non maîtrisés.
