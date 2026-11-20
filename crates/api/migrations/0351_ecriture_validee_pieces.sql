-- Liens de pièces d'une écriture validée (L2-26, PR 13b ; ADR 0015 : lien verrouillé à la
-- validation). Dépend de `piece` (`0300_pieces.sql`, voie pièces) et de `ecriture_validee`
-- (`0203_validation.sql`) : numéro supérieur à `0300`, hors de la plage de la voie comptabilité,
-- comme `0350_brouillard_pieces.sql` (README du crate, « Migrations »).
--
-- Ajout seul : le lien copié à la validation ne se retire ni ne se modifie. Une pièce liée à une
-- écriture validée ne s'écarte plus (`crate::base::brouillard::brouillons_lies_a_la_piece` compte
-- aussi ces liens). Le rôle est celui qu'avait le lien au brouillon ; il n'entre pas dans le
-- contenu haché (le contenu canonique ne cite que les empreintes des pièces).
-- Aucune colonne d'instant : celui de la validation est dans `ecriture_validee`.
CREATE TABLE ecriture_validee_piece (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    ecriture_id uuid NOT NULL,
    piece_id uuid NOT NULL,
    -- Rang du lien dans la justification hachée (ordre des références `piece=`).
    position smallint NOT NULL CHECK (position BETWEEN 1 AND 200),
    role text NOT NULL CHECK (role IN ('principale', 'complementaire')),
    PRIMARY KEY (cabinet_id, dossier_id, ecriture_id, piece_id),
    UNIQUE (cabinet_id, dossier_id, ecriture_id, position),
    FOREIGN KEY (cabinet_id, dossier_id, ecriture_id)
        REFERENCES ecriture_validee (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, piece_id)
        REFERENCES piece (cabinet_id, dossier_id, id)
);
CREATE UNIQUE INDEX ecriture_validee_piece_principale
    ON ecriture_validee_piece (cabinet_id, dossier_id, ecriture_id) WHERE role = 'principale';
-- Recherche des écritures liées à une pièce (refus de l'écart).
CREATE INDEX ecriture_validee_piece_par_piece
    ON ecriture_validee_piece (cabinet_id, dossier_id, piece_id);

ALTER TABLE ecriture_validee_piece ENABLE ROW LEVEL SECURITY;
ALTER TABLE ecriture_validee_piece FORCE ROW LEVEL SECURITY;
CREATE POLICY ecriture_validee_piece_tenant ON ecriture_validee_piece
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
GRANT SELECT, INSERT ON ecriture_validee_piece TO sigma_application;
