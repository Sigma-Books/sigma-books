-- Fiche du tiers, colonnes ajoutées à `tiers` (lot L3, PR 7a, L3-15 ; ADR 0031 points 1, 4, 5).
--
-- Voie « tiers et trésorerie », sous-plage 0500 à 0509 (crates/api/README.md, « Migrations ») :
-- `tiers` vient de `0200_referentiel.sql`, plage inférieure déjà fusionnée.
--
-- - `code` : code auxiliaire du tiers, **unique dans le dossier** (bloquant, comme Tryton
--   `party/party.py:107`) et **immuable** (hors du droit d'`UPDATE`). Alphabet `A-Z 0-9 . _ -`,
--   1 à 20 caractères (ADR 0031 point 1). Les tiers de L2 reçoivent ici `T` suivi de leur rang
--   dans le dossier sur six chiffres (ordre de création, puis identifiant), la forme que l'API
--   tire pour un tiers créé sans code.
-- - `devise` : devise par défaut de saisie (code ISO 4217 à trois lettres ; un tiers étranger
--   peut avoir une devise hors du pack), facultative.
-- - `assujetti_tva`, `soumis_retenue` : indicateurs stockés, **sans effet comptable en L3**
--   (ADR 0031 point 4, plan C12).
-- - Archivage (ADR 0031 point 5) : `actif` (L2) faux, avec motif, instant et auteur. Un tiers de
--   L2 désactivé n'a pas de motif : la contrainte n'exige le motif qu'avec l'instant.
-- `condition_reglement_id` (ADR 0033) est ajoutée par la PR 8 avec sa table (`0510`–`0519`).
--
-- `type` et `compte_collectif` restent figés (choix Sigma de L2, ADR 0031 point 1) ; `code` est
-- immuable : aucun des trois n'entre dans le droit d'`UPDATE`.

ALTER TABLE tiers
    ADD COLUMN code text,
    ADD COLUMN devise text CHECK (devise ~ '^[A-Z]{3}$'),
    ADD COLUMN assujetti_tva boolean NOT NULL DEFAULT false,
    ADD COLUMN soumis_retenue boolean NOT NULL DEFAULT false,
    ADD COLUMN motif_archivage text CHECK (
        length(motif_archivage) <= 200 AND length(btrim(motif_archivage)) >= 1
    ),
    ADD COLUMN archive_le timestamptz,
    ADD COLUMN archive_par uuid;

-- Le propriétaire est soumis à la RLS forcée : elle est levée le temps de numéroter les tiers
-- existants de tous les dossiers, puis rétablie dans la même transaction de migration.
ALTER TABLE tiers NO FORCE ROW LEVEL SECURITY;
UPDATE tiers t SET code = 'T' || lpad(r.rang::text, 6, '0')
FROM (
    SELECT cabinet_id, dossier_id, id,
           row_number() OVER (PARTITION BY cabinet_id, dossier_id ORDER BY cree_le, id) AS rang
    FROM tiers
) r
WHERE r.cabinet_id = t.cabinet_id AND r.dossier_id = t.dossier_id AND r.id = t.id;
ALTER TABLE tiers FORCE ROW LEVEL SECURITY;

ALTER TABLE tiers
    ALTER COLUMN code SET NOT NULL,
    ADD CONSTRAINT tiers_code_forme CHECK (code ~ '^[A-Z0-9._-]{1,20}$'),
    ADD CONSTRAINT tiers_code_unique UNIQUE (cabinet_id, dossier_id, code),
    -- Archivé : `actif` faux ; motif, instant et auteur vont ensemble.
    ADD CONSTRAINT tiers_archivage CHECK (
        (archive_le IS NULL) = (archive_par IS NULL)
        AND (archive_le IS NULL) = (motif_archivage IS NULL)
        AND (archive_le IS NULL OR NOT actif)
    );

GRANT UPDATE (devise, assujetti_tva, soumis_retenue, motif_archivage, archive_le, archive_par)
    ON tiers TO sigma_application;
