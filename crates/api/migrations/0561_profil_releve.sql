-- Profils d'import de relevé du dossier (L3-22, PR 12a de `docs/lots/L3.md` ; ADR 0034 point 6).
--
-- Un profil est une **donnée** (encodage, délimiteur, guillemet, lignes d'en-tête et de pied,
-- format de date, séparateurs, colonnes, signe inversé : Tryton `StatementImportCSV`,
-- `account_statement/statement.py:1449-1576` ; OCA `account_statement_import_sheet_mapping.py`),
-- validée par `formats` à la création. **Versionné en ajout seul**, comme les versions de
-- paramètres fiscaux d'un dossier (ADR 0030, `0208`) : une version publiée ne change jamais ;
-- tout changement (nom compris) est une **nouvelle version** du même profil. Un import (PR 12b)
-- citera le profil **et sa version**.
--
-- `definition` : forme JSON canonique du profil validé (`formats::releve::ProfilBrut`, ordre des
-- champs fixe), relue et revalidée à chaque usage ; `empreinte` : son SHA-256, citée par l'audit
-- (jamais la définition elle-même). Aucun montant ici.
CREATE TABLE profil_releve (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    version integer NOT NULL CHECK (version >= 1),
    nom text NOT NULL CHECK (
        length(nom) <= 200 AND length(btrim(nom)) >= 1 AND nom !~ '[[:cntrl:]]'
    ),
    definition text NOT NULL CHECK (octet_length(definition) BETWEEN 2 AND 65536),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    cree_par uuid NOT NULL REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id, version),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    -- Une version n > 1 suit la version n - 1 du même profil (aucun trou, aucune version
    -- orpheline).
    FOREIGN KEY (cabinet_id, dossier_id, id, version_precedente)
        REFERENCES profil_releve (cabinet_id, dossier_id, id, version),
    version_precedente integer GENERATED ALWAYS AS (NULLIF(version - 1, 0)) STORED
);

ALTER TABLE profil_releve ENABLE ROW LEVEL SECURITY;
ALTER TABLE profil_releve FORCE ROW LEVEL SECURITY;
CREATE POLICY profil_releve_tenant ON profil_releve
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Ajout seul (catalogue `releves.toml`) : ni UPDATE ni DELETE.
GRANT SELECT, INSERT ON profil_releve TO sigma_application;
