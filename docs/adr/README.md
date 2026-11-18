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
| 0011 | Persistance et multi-tenant : rôles PostgreSQL, RLS forcée, isolation à trois niveaux | Proposée |
| 0012 | Sessions, secours, récupération de compte et création des cabinets | Proposée |
| 0013 | Chiffrement : disques des serveurs obligatoire, chiffrement par cabinet facultatif | Proposée |
| 0014 | Journal d'audit chaîné | Proposée |
| 0015 | Pièces justificatives en L2 | Proposée |

Modèle : copier `0000-modele.md`.
