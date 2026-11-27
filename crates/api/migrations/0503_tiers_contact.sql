-- Moyens de contact d'un tiers (lot L3, PR 7a, L3-15 ; ADR 0031 point 3).
--
-- Types repris de Tryton (`party/contact_mechanism.py`, `_TYPES` : téléphone, mobile, fax,
-- courriel, site web, autre). Valeur libre bornée : la forme d'un courriel est seulement
-- **signalée** par l'API (Tryton refuse, `:357-369`, non repris), le téléphone est un texte libre.
-- `nom` : personne à joindre, facultatif. Régime C : `UPDATE` et `DELETE` accordés, chaque
-- mutation journalisée par l'API.

CREATE TABLE tiers_contact (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    tiers_id uuid NOT NULL,
    type text NOT NULL CHECK (
        type IN ('telephone', 'mobile', 'fax', 'courriel', 'site_web', 'autre')
    ),
    valeur text NOT NULL CHECK (length(valeur) <= 200 AND length(btrim(valeur)) >= 1),
    nom text CHECK (length(nom) <= 200 AND length(btrim(nom)) >= 1),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, tiers_id) REFERENCES tiers (cabinet_id, dossier_id, id)
);
CREATE INDEX tiers_contact_tiers ON tiers_contact (cabinet_id, dossier_id, tiers_id);

ALTER TABLE tiers_contact ENABLE ROW LEVEL SECURITY;
ALTER TABLE tiers_contact FORCE ROW LEVEL SECURITY;
CREATE POLICY tiers_contact_tenant ON tiers_contact
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT, DELETE ON tiers_contact TO sigma_application;
GRANT UPDATE (type, valeur, nom) ON tiers_contact TO sigma_application;
