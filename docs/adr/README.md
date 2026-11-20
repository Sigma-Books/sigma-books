# Décisions d'architecture (ADR)

Chaque décision structurante est consignée dans un fichier `NNNN-titre.md` : contexte, décision, conséquences. Une décision n'est jamais modifiée après acceptation : on la remplace par un nouvel ADR qui la déclare caduque.

| N° | Décision | Statut |
|---|---|---|
| 0001 | Rust pour le backend et le moteur comptable | Acceptée |
| 0002 | Interface SvelteKit statique, site public Astro | Acceptée |
| 0003 | Monolithe modulaire et workers isolés pour les documents | Acceptée |
| 0004 | PostgreSQL : RLS, rôles séparés, file de tâches intégrée | Acceptée |
| 0005 | Autorisation par politiques Cedar, refus par défaut | Acceptée |
| 0006 | Authentification par passkeys, TOTP en secours, jamais de SMS | Acceptée |
| 0007 | Écriture validée = définitive ; intégrité par chaîne de hachage | Acceptée |
| 0008 | L'IA propose, l'humain valide | Acceptée |
| 0009 | Licence AGPL-3.0-or-later, CLA, marque réservée | Acceptée |
| 0010 | Horodatage RFC 3161 de la tête de chaîne : DigiCert, Sectigo en secours | Acceptée |
| 0011 | Persistance et multi-tenant : rôles PostgreSQL, RLS forcée, isolation à trois niveaux | Acceptée |
| 0012 | Sessions, secours, récupération de compte et création des cabinets | Acceptée |
| 0013 | Chiffrement : disques des serveurs obligatoire, chiffrement par cabinet facultatif | Acceptée |
| 0014 | Journal d'audit chaîné | Acceptée |
| 0015 | Pièces justificatives en L2 | Acceptée |
| 0016 | Consultation de l'audit : la portée cabinet est réservée à l'administrateur (précise l'ADR 0014) | Acceptée |
| 0017 | Événements d'authentification hors cabinet et conservation des sessions (complète les ADR 0012 et 0014) | Acceptée |
| 0018 | Passkeys : copie corrigée de `crypto-glue` en attendant sa version 0.2.3 | Acceptée |
| 0019 | Étiquette fiscale de la ligne d'écriture : dans le contenu haché, régimes et motifs sourcés (écriture v3, export v4) | Proposée |
| 0020 | Validation unitaire : ordre des verrous, séparation des tâches relue, échelle des montants du XOF | Proposée |
| 0021 | Validation par lot et contre-passation : tout ou rien, une contre-passation par origine | Proposée |

Modèle : copier `0000-modele.md`.
