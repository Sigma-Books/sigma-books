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

Modèle : copier `0000-modele.md`.
