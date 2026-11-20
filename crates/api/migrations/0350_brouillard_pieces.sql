-- Lien N-N entre un brouillon et ses pièces (L2-25, PR 13a ; spec §6.5 ; ADR 0015).
--
-- Numéro hors de la plage `0200` à `0299` de la voie comptabilité, volontairement : cette table
-- dépend de `piece` (`0300_pieces.sql`, voie pièces), et `crates/api/README.md` (« Migrations »)
-- veut qu'une dépendance entre voies passe par une migration de numéro supérieur à celle dont
-- elle dépend. Sur une base neuve, `0350` s'applique donc après `0300` ; elle dépend aussi de
-- `0202_brouillard.sql`.
--
-- Aucune colonne d'instant (fil-piège des tables exemptées) : l'instant du lien est dans le
-- journal d'audit chaîné.
--
-- Le lien est supprimable (retrait d'une pièce d'un brouillon, suppression du brouillon). À la
-- validation (PR 13b), il est copié dans les liens de l'écriture validée, qui eux sont en ajout
-- seul. Une pièce écartée ne se lie pas ; une pièce liée à un brouillon ne s'écarte pas
-- (contrôles de `api`, sous verrou de la pièce).

CREATE TABLE brouillon_piece (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    brouillon_id uuid NOT NULL,
    piece_id uuid NOT NULL,
    role text NOT NULL CHECK (role IN ('principale', 'complementaire')),
    lie_par uuid NOT NULL REFERENCES utilisateur (id),
    PRIMARY KEY (cabinet_id, dossier_id, brouillon_id, piece_id),
    CONSTRAINT brouillon_piece_brouillon_fk FOREIGN KEY (cabinet_id, dossier_id, brouillon_id)
        REFERENCES brouillon (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, piece_id)
        REFERENCES piece (cabinet_id, dossier_id, id)
);
-- Au plus une pièce principale par brouillon.
CREATE UNIQUE INDEX brouillon_piece_principale
    ON brouillon_piece (cabinet_id, dossier_id, brouillon_id) WHERE role = 'principale';
-- Recherche des brouillons liés à une pièce (refus de l'écart).
CREATE INDEX brouillon_piece_par_piece ON brouillon_piece (cabinet_id, dossier_id, piece_id);

ALTER TABLE brouillon_piece ENABLE ROW LEVEL SECURITY;
ALTER TABLE brouillon_piece FORCE ROW LEVEL SECURITY;
CREATE POLICY brouillon_piece_tenant ON brouillon_piece
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT, DELETE ON brouillon_piece TO sigma_application;
