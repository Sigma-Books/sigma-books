-- Journal d'audit chaîné (ADR 0014, ADR 0016 ; docs/integrite/audit-v1.md ; L2 PR 7).
--
-- Deux portées, donc deux chaînes : une par cabinet (administration, droits, création des
-- dossiers) et une par dossier. Chacune a sa table d'entrées, en AJOUT SEUL, et sa table de
-- tête, état dérivé verrouillé `FOR UPDATE` (ADR 0011). Deux tables par portée plutôt qu'une
-- table commune : la politique RLS d'une table de dossier exige le dossier du contexte
-- (`catalogue.toml`), ce qu'une entrée du cabinet, sans dossier, ne peut satisfaire.
--
-- Une entrée stocke son contenu canonique (le texte haché) et son empreinte ; rien d'autre :
-- aucune colonne redondante qui pourrait diverger du texte haché. Unicité (portée,
-- identifiant, rang) : la clé primaire de chaque table d'entrées.
--
-- Droits :
-- - `sigma_application` : INSERT seul sur les entrées (ni SELECT, ni UPDATE, ni DELETE, F2.6) ;
--   sur les têtes, SELECT et INSERT (création au premier événement de la portée) et UPDATE des
--   seules colonnes de la tête, que `SELECT … FOR UPDATE` exige ; aucun DELETE ;
-- - `sigma_lecture_audit` : SELECT seul, sur ces quatre tables seulement, sous RLS forcée et
--   contexte du tenant.
--
-- Ordre des verrous d'une action (ADR 0014) : ceux de l'action, puis la tête d'audit du
-- cabinet, puis celle du dossier.

-- Entrées de la chaîne d'un cabinet.
CREATE TABLE audit_cabinet (
    cabinet_id uuid NOT NULL REFERENCES cabinet (id),
    rang bigint NOT NULL CHECK (rang >= 1),
    contenu text NOT NULL CHECK (octet_length(contenu) <= 4194304),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    PRIMARY KEY (cabinet_id, rang)
);

ALTER TABLE audit_cabinet ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_cabinet FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_cabinet_tenant ON audit_cabinet
    FOR ALL
    USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);

GRANT INSERT ON audit_cabinet TO sigma_application;
GRANT SELECT ON audit_cabinet TO sigma_lecture_audit;

-- Tête de la chaîne d'un cabinet : dernier rang et dernière empreinte (empreinte de départ et
-- rang 0 avant la première entrée).
CREATE TABLE audit_tete_cabinet (
    cabinet_id uuid PRIMARY KEY REFERENCES cabinet (id),
    rang bigint NOT NULL CHECK (rang >= 0),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32)
);

ALTER TABLE audit_tete_cabinet ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_tete_cabinet FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_tete_cabinet_tenant ON audit_tete_cabinet
    FOR ALL
    USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);

GRANT SELECT, INSERT ON audit_tete_cabinet TO sigma_application;
GRANT UPDATE (rang, empreinte) ON audit_tete_cabinet TO sigma_application;
GRANT SELECT ON audit_tete_cabinet TO sigma_lecture_audit;

-- Entrées de la chaîne d'un dossier : visibles dans le contexte de ce dossier seulement.
CREATE TABLE audit_dossier (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    rang bigint NOT NULL CHECK (rang >= 1),
    contenu text NOT NULL CHECK (octet_length(contenu) <= 4194304),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    PRIMARY KEY (cabinet_id, dossier_id, rang),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id)
);

ALTER TABLE audit_dossier ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_dossier FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_dossier_tenant ON audit_dossier
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT INSERT ON audit_dossier TO sigma_application;
GRANT SELECT ON audit_dossier TO sigma_lecture_audit;

-- Tête de la chaîne d'un dossier.
CREATE TABLE audit_tete_dossier (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    rang bigint NOT NULL CHECK (rang >= 0),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    PRIMARY KEY (cabinet_id, dossier_id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id)
);

ALTER TABLE audit_tete_dossier ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_tete_dossier FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_tete_dossier_tenant ON audit_tete_dossier
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT ON audit_tete_dossier TO sigma_application;
GRANT UPDATE (rang, empreinte) ON audit_tete_dossier TO sigma_application;
GRANT SELECT ON audit_tete_dossier TO sigma_lecture_audit;
