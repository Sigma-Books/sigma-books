-- Tables d'organisation : cabinet, utilisateur, appartenance, dossier, accès aux dossiers (ADR 0011).
--
-- Exécutée par `sigma_migration`, propriétaire de la base (crates/api/sql/approvisionnement.sql),
-- jamais par le binaire `api`. Les rôles existent déjà : ils relèvent de l'approvisionnement.
--
-- Règles communes (vérifiées par le test de catalogue, crates/api/tests/catalogue.rs) :
-- - toute table a la RLS activée et forcée, et une politique `USING` et `WITH CHECK` pour
--   toutes les commandes, sauf les tables d'instance sans RLS de `catalogue.toml` ;
-- - les politiques lisent `NULLIF(current_setting('sigma.cabinet', true), '')::uuid` :
--   contexte absent => aucune ligne (une comparaison à NULL n'est jamais vraie) ;
-- - une table qui porte `dossier_id` se restreint au dossier du contexte ; seules les tables
--   listées dans `catalogue.toml` (`dossier_facultatif`) voient tout le cabinet sans dossier ;
-- - clés étrangères composites (`cabinet_id`, `id`), index en tête sur `cabinet_id` ;
-- - identifiants UUID v4 tirés par `api` (aucune valeur par défaut), instants en `timestamptz`.

-- Schéma : seuls les rôles de Sigma Books y accèdent, et aucun d'eux n'y crée d'objet.
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO sigma_application, sigma_lecture_audit;

-- Cabinet : le tenant. Créé par la commande d'administration de l'instance (ADR 0012, Q4a).
CREATE TABLE cabinet (
    id uuid PRIMARY KEY,
    nom text NOT NULL CHECK (length(btrim(nom)) BETWEEN 1 AND 200),
    -- Séparation des tâches (spec §20, Q5.3) : activée par défaut ; la désactivation exige
    -- ré-authentification, audit et alerte (ADR 0014).
    separation_taches boolean NOT NULL DEFAULT true,
    cree_le timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE cabinet ENABLE ROW LEVEL SECURITY;
ALTER TABLE cabinet FORCE ROW LEVEL SECURITY;
CREATE POLICY cabinet_tenant ON cabinet
    FOR ALL
    USING (id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);

GRANT SELECT, INSERT ON cabinet TO sigma_application;
GRANT UPDATE (nom, separation_taches) ON cabinet TO sigma_application;

-- Utilisateur : table d'instance (identité, ADR 0011 et 0012) ; un utilisateur peut appartenir
-- à plusieurs cabinets (spec §4.1), il n'a donc pas de `cabinet_id`. Facteurs et sessions : PR 8.
-- Aucune donnée comptable. Sous RLS malgré tout (politiques après `appartenance`).
CREATE TABLE utilisateur (
    id uuid PRIMARY KEY,
    courriel text NOT NULL CHECK (length(courriel) BETWEEN 3 AND 254 AND position('@' IN courriel) > 1),
    nom text NOT NULL CHECK (length(btrim(nom)) BETWEEN 1 AND 200),
    cree_le timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX utilisateur_courriel_unique ON utilisateur (lower(courriel));

-- Appartenance d'un utilisateur à un cabinet, avec son rôle par défaut (spec §20, hors client).
CREATE TABLE appartenance (
    cabinet_id uuid NOT NULL REFERENCES cabinet (id),
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    role text NOT NULL CHECK (role IN ('administrateur', 'valideur', 'collaborateur', 'auditeur')),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, utilisateur_id)
);

ALTER TABLE appartenance ENABLE ROW LEVEL SECURITY;
ALTER TABLE appartenance FORCE ROW LEVEL SECURITY;
CREATE POLICY appartenance_tenant ON appartenance
    FOR ALL
    USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);

GRANT SELECT, INSERT, DELETE ON appartenance TO sigma_application;
GRANT UPDATE (role) ON appartenance TO sigma_application;

-- RLS de `utilisateur` : sont visibles l'utilisateur du contexte et les membres du cabinet du
-- contexte (la sous-requête lit `appartenance`, elle-même sous RLS : limitée à ce cabinet).
-- Sans contexte, aucune ligne. Aucun UPDATE ni DELETE accordé : aucune fonction ne s'en sert.
-- Création (invitation, PR 9) : seulement dans le contexte d'un cabinet ; la ligne créée n'est
-- visible qu'une fois l'appartenance insérée (pas de `RETURNING` sur l'insertion).
-- La recherche par courriel à la connexion, sans contexte de cabinet, sera conçue en PR 8
-- (ADR 0012), par exemple par une fonction dédiée et auditée : elle n'élargit pas cette lecture.
ALTER TABLE utilisateur ENABLE ROW LEVEL SECURITY;
ALTER TABLE utilisateur FORCE ROW LEVEL SECURITY;
CREATE POLICY utilisateur_lecture ON utilisateur
    FOR SELECT
    USING (
        id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
        OR EXISTS (SELECT 1 FROM appartenance a WHERE a.utilisateur_id = utilisateur.id)
    );
CREATE POLICY utilisateur_creation ON utilisateur
    FOR INSERT
    WITH CHECK (NULLIF(current_setting('sigma.cabinet', true), '') IS NOT NULL);
CREATE POLICY utilisateur_modification ON utilisateur
    FOR UPDATE
    USING (id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid)
    WITH CHECK (id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);
CREATE POLICY utilisateur_suppression ON utilisateur
    FOR DELETE
    USING (id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);

GRANT SELECT, INSERT ON utilisateur TO sigma_application;

-- Dossier : une entité juridique tenue par le cabinet (spec §4.1).
-- Sans dossier en contexte (action du cabinet), tous les dossiers du cabinet sont visibles ;
-- avec un dossier en contexte, ce dossier seulement (cloisonnement, spec §2 principe 12).
CREATE TABLE dossier (
    cabinet_id uuid NOT NULL REFERENCES cabinet (id),
    id uuid NOT NULL UNIQUE,
    nom text NOT NULL CHECK (length(btrim(nom)) BETWEEN 1 AND 200),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, id)
);

ALTER TABLE dossier ENABLE ROW LEVEL SECURITY;
ALTER TABLE dossier FORCE ROW LEVEL SECURITY;
CREATE POLICY dossier_tenant ON dossier
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND (NULLIF(current_setting('sigma.dossier', true), '') IS NULL
             OR id = NULLIF(current_setting('sigma.dossier', true), '')::uuid)
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND (NULLIF(current_setting('sigma.dossier', true), '') IS NULL
             OR id = NULLIF(current_setting('sigma.dossier', true), '')::uuid)
    );

GRANT SELECT, INSERT ON dossier TO sigma_application;
GRANT UPDATE (nom) ON dossier TO sigma_application;

-- Affectation explicite d'un membre du cabinet à un dossier (Q5.1), avec la délégation
-- du droit de valider accordée par dossier (Q5.2, spec §20 « selon délégation »).
-- Vérifiée par Cedar et par la requête, pas par la RLS (ADR 0011).
-- Les clés composites interdisent d'affecter un dossier ou un membre d'un autre cabinet.
CREATE TABLE acces_dossier (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    utilisateur_id uuid NOT NULL,
    validation_deleguee boolean NOT NULL DEFAULT false,
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, utilisateur_id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, utilisateur_id) REFERENCES appartenance (cabinet_id, utilisateur_id)
);
CREATE INDEX acces_dossier_utilisateur ON acces_dossier (cabinet_id, utilisateur_id);

ALTER TABLE acces_dossier ENABLE ROW LEVEL SECURITY;
ALTER TABLE acces_dossier FORCE ROW LEVEL SECURITY;
CREATE POLICY acces_dossier_tenant ON acces_dossier
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND (NULLIF(current_setting('sigma.dossier', true), '') IS NULL
             OR dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid)
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND (NULLIF(current_setting('sigma.dossier', true), '') IS NULL
             OR dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid)
    );

GRANT SELECT, INSERT, DELETE ON acces_dossier TO sigma_application;
GRANT UPDATE (validation_deleguee) ON acces_dossier TO sigma_application;
