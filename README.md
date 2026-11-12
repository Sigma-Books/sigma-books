# Sigma Books

**Logiciel comptable SYSCOHADA révisé, piloté par l'IA, sécurisé dès la conception.**

Sigma Books lit les pièces justificatives (factures, reçus, relevés bancaires et mobile money), prépare les écritures, rapproche la trésorerie et produit les états financiers SYSCOHADA. Le comptable contrôle et valide.

> 🚧 Projet en cours de construction. Rien n'est encore utilisable en production.

## Principes
- **Conformité** : SYSCOHADA révisé (AUDCIF 2017), Système normal. Sénégal en premier.
- **L'IA propose, l'humain valide** : aucune écriture n'est validée sans action humaine.
- **Intégrité prouvable** : écritures validées définitives, chaîne de hachage, outil de vérification public.
- **Security by design** : Rust, refus par défaut, isolation multi-tenant, documents traités en bac à sable.

## Documentation
- [Spécification fonctionnelle](docs/specification.md)
- [Fondations techniques](docs/fondations.md)
- [Décisions d'architecture](docs/adr/)

## Sécurité
Voir [SECURITY.md](SECURITY.md). Ne signalez jamais une vulnérabilité dans une issue publique.

## Licence
Code sous [AGPL-3.0-or-later](LICENSE). Le nom « Sigma Books » et le logo sont des marques réservées : voir [TRADEMARK.md](TRADEMARK.md).
