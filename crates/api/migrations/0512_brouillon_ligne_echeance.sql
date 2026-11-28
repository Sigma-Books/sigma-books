-- Échéance facultative d'une ligne de brouillon (L3-17, PR 8 ; ADR 0033 point 1).
--
-- Colonne ajoutée à une table existante (`0202_brouillard.sql`, plage inférieure déjà fusionnée) :
-- la date d'échéance d'une **ligne de tiers**, saisie au brouillard, copiée dans `echeance_ligne`
-- (`0513`) à la validation. Elle n'entre **pas** dans le contenu haché (formats v1 à v5 figés,
-- `docs/integrite/`), comme chez Odoo (`account_move_line.py:3416-3422`). Une ligne sans tiers
-- n'a pas d'échéance (ADR 0033 point 1 : 422 `echeance_sans_tiers` à la saisie, contrainte en
-- seconde barrière).
--
-- Ce n'est pas une date comptable : la clôture d'une période lit la date de l'en-tête
-- (`brouillon`), jamais celle-ci (catalogue `echeancier.toml`, `brouillard_colonne_non_comptable`).
ALTER TABLE brouillon_ligne
    ADD COLUMN date_echeance date,
    ADD CONSTRAINT brouillon_ligne_echeance_tiers
        CHECK (date_echeance IS NULL OR tiers_id IS NOT NULL);
