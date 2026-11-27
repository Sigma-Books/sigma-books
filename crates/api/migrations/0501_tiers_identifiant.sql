-- Identifiants légaux d'un tiers (lot L3, PR 7a, L3-15 ; ADR 0031 point 2, ADR 0027).
--
-- Liste typée (Tryton `party.identifier`, `party/party.py:832`) : les types sont ceux du pack du
-- pays **du dossier** (`crates/syscohada/data/pays.toml`), contrôlés par l'API ; la base ne
-- connaît aucun type de pays (aucune règle de pays en dur). Au plus un identifiant par type et
-- par tiers. Aucun format imposé (aucun n'est sourcé) ; un doublon chez un autre tiers est
-- **signalé** par l'API, jamais refusé (Tryton `:1061-1078`) : `valeur_normalisee` sert à cette
-- recherche (majuscules ; blancs, espaces insécables et caractères de format Unicode retirés :
-- l'API refuse déjà ces derniers, la base les retire encore, F11.8).
--
-- Régime « configuration modifiable et journalisée » (C du plan) : `UPDATE` de la valeur et
-- `DELETE` accordés ; chaque mutation passe par l'API, qui l'inscrit à l'audit chaîné (noms de
-- champs, jamais de valeur).

CREATE TABLE tiers_identifiant (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    tiers_id uuid NOT NULL,
    type text NOT NULL CHECK (type ~ '^[A-Z][A-Z0-9_]{1,31}$'),
    valeur text NOT NULL CHECK (length(valeur) <= 64 AND length(btrim(valeur)) >= 1),
    -- Majuscules ; blancs (espaces insécables compris) et caractères de format Unicode retirés.
    valeur_normalisee text GENERATED ALWAYS AS (upper(regexp_replace(valeur,
        '[[:space:]\u00a0\u1680\u2000-\u200f\u2028-\u202f\u205f-\u2064\u2066-\u206f\u3000'
        || '\ufeff\u00ad\u0600-\u0605\u061c\u06dd\u070f\u0890-\u0891\u08e2\u180e\ufff9-\ufffb\U000e0001\U000e0020-\U000e007f]', '', 'g'))) STORED,
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    UNIQUE (cabinet_id, dossier_id, tiers_id, type),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, tiers_id) REFERENCES tiers (cabinet_id, dossier_id, id)
);
CREATE INDEX tiers_identifiant_doublon
    ON tiers_identifiant (cabinet_id, dossier_id, type, valeur_normalisee);

ALTER TABLE tiers_identifiant ENABLE ROW LEVEL SECURITY;
ALTER TABLE tiers_identifiant FORCE ROW LEVEL SECURITY;
CREATE POLICY tiers_identifiant_tenant ON tiers_identifiant
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT, DELETE ON tiers_identifiant TO sigma_application;
GRANT UPDATE (valeur) ON tiers_identifiant TO sigma_application;
