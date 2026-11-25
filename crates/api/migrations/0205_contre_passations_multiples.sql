-- Contre-passations multiples (L2-27, PR 13c révisée ; ADR 0021 point 4, décision du mainteneur
-- du 2026-11-25 : suivre Tryton et Odoo). Voie comptabilité, plage 0200 à 0299. Dépend de
-- `0204_contre_passation.sql`, qui reste telle que fusionnée.
--
-- Une écriture validée peut être contre-passée plusieurs fois : l'unicité de l'origine est levée.
-- La sérialisation (numéros, chaîne) reste celle de la tête de chaîne du dossier. Un index non
-- unique sert les lectures « contre-passations de cette écriture » (`origine_id`).
DROP INDEX ecriture_validee_origine_unique;

CREATE INDEX ecriture_validee_origine_idx
    ON ecriture_validee (cabinet_id, dossier_id, origine_id)
    WHERE origine_id IS NOT NULL;
