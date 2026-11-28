-- Type de pièce « texte » d'un relevé CSV (L3-22, PR 12a de `docs/lots/L3.md` ; ADR 0034 point 4,
-- qui amende l'ADR 0015, « Types admis »).
--
-- Un relevé CSV déposé est une **pièce externe** (WORM, empreinte SHA-256 du serveur, rétention du
-- paquet du pays : ADR 0015) de type `text/csv`. Il n'a pas de signature : `api` le reconnaît par
-- son **contenu** (aucun octet nul, aucun caractère de contrôle hors tabulation et fins de ligne,
-- décodable dans un encodage de la liste fermée de `formats`, longueur de ligne bornée), et
-- seulement par la route de dépôt de relevé, jamais par le dépôt général des pièces (F11.4).
-- Le fichier n'est jamais rendu : téléchargement en pièce jointe, `application/octet-stream`.
--
-- La migration fusionnée `0300` n'est pas modifiée : la contrainte de colonne est remplacée ici
-- (README du crate, « Migrations »). Le type texte est réservé aux pièces externes ; la note
-- d'opérations diverses reste la seule pièce interne (`text/plain; charset=utf-8`, contrainte
-- de table de `0300`, inchangée).
ALTER TABLE piece DROP CONSTRAINT piece_type_contenu_check;
ALTER TABLE piece ADD CONSTRAINT piece_type_contenu_check CHECK (type_contenu IN (
    'application/pdf', 'image/jpeg', 'image/png', 'text/plain; charset=utf-8', 'text/csv'
));
