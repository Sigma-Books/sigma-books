# crate `worker-documents` — traitement isolé des documents

**Rôle :** traiter les fichiers déposés (non fiables) : détection du type réel, limites, antivirus, rasterisation des PDF en images, compression. Renvoie images et métadonnées.

**Fondations :** §3.8.

## Règles propres
- S'exécute dans un **bac à sable** : aucun réseau, aucun accès à la base, système de fichiers éphémère, seccomp/gVisor/nsjail.
- Communication avec l'API uniquement par messages signés via la file.
- Les bibliothèques de rendu PDF (souvent en C/C++) ne sont appelées **que** dans ce worker.
- Limites strictes : taille, nombre de pages, dimensions d'image, temps d'exécution.
- Fuzzing des entrées.
