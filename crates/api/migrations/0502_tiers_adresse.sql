-- Adresses d'un tiers (lot L3, PR 7a, L3-15 ; ADR 0031 point 3).
--
-- Champs facultatifs (Tryton `party/address.py:25-137` : rue, code postal, ville, pays,
-- subdivision), textes bornés sans caractère de contrôle ni de format (contrôlés par l'API, même
-- grammaire que `TexteCourt` : F11.8). `pays` : code ISO 3166-1 alpha-2 de **forme** seulement
-- (un tiers peut être hors de l'OHADA) ; son changement est journalisé (ADR 0031 point 2).
-- Régime C : `UPDATE` et `DELETE` accordés, chaque mutation journalisée par l'API.

CREATE TABLE tiers_adresse (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    tiers_id uuid NOT NULL,
    libelle text CHECK (length(libelle) <= 100 AND length(btrim(libelle)) >= 1),
    rue text CHECK (length(rue) <= 200 AND length(btrim(rue)) >= 1),
    code_postal text CHECK (length(code_postal) <= 20 AND length(btrim(code_postal)) >= 1),
    ville text CHECK (length(ville) <= 100 AND length(btrim(ville)) >= 1),
    region text CHECK (length(region) <= 100 AND length(btrim(region)) >= 1),
    pays text CHECK (pays ~ '^[A-Z]{2}$'),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, tiers_id) REFERENCES tiers (cabinet_id, dossier_id, id)
);
CREATE INDEX tiers_adresse_tiers ON tiers_adresse (cabinet_id, dossier_id, tiers_id);

ALTER TABLE tiers_adresse ENABLE ROW LEVEL SECURITY;
ALTER TABLE tiers_adresse FORCE ROW LEVEL SECURITY;
CREATE POLICY tiers_adresse_tenant ON tiers_adresse
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT, DELETE ON tiers_adresse TO sigma_application;
GRANT UPDATE (libelle, rue, code_postal, ville, region, pays) ON tiers_adresse TO sigma_application;
