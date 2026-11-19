-- Sessions (ADR 0012 ; L2-17, PR 8a) et lecture de ses propres cabinets sans cabinet en contexte.
--
-- Exécutée par `sigma_migration` (crates/api/README.md). Règles communes : migration 0001.

-- Session : table d'instance (un utilisateur, aucun cabinet ; catalogue.toml). Le jeton n'est
-- jamais stocké : seule son empreinte SHA-256 l'est (ADR 0012, F1.2). L'identifiant `id` est
-- public : il est cité au journal d'audit (`session=`, ADR 0014), jamais le jeton.
-- Instants posés par `api` (horloge injectable dans les tests) : aucune valeur par défaut.
CREATE TABLE session (
    id uuid PRIMARY KEY,
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    jeton_hache bytea NOT NULL UNIQUE CHECK (length(jeton_hache) = 32),
    -- `complete` : accès aux routes métier ; `restreinte` (code de récupération) et `enrolement`
    -- (lien d'invitation ou de réinitialisation) : enrôlement d'une passkey seulement (ADR 0012).
    nature text NOT NULL CHECK (nature IN ('complete', 'restreinte', 'enrolement')),
    -- Facteur vérifié à l'ouverture (PR 8b et 8c ; lien d'enrôlement : PR 9).
    methode text NOT NULL
        CHECK (methode IN ('passkey', 'secours', 'code_recuperation', 'lien_enrolement')),
    cree_le timestamptz NOT NULL,
    derniere_activite timestamptz NOT NULL,
    derniere_verification timestamptz NOT NULL,
    revoquee_le timestamptz,
    -- Motifs de la PR 8a ; les PR suivantes en ajoutent (changement de facteur ou de droits,
    -- réinitialisation, administrateur).
    motif_revocation text CHECK (motif_revocation IN ('deconnexion', 'revocation_utilisateur')),
    -- Description de l'appareil (navigateur), saisie libre bornée ; adresse à titre de trace
    -- seulement : la session n'est pas liée à l'adresse.
    appareil text CHECK (length(appareil) <= 200),
    adresse inet,
    CHECK ((revoquee_le IS NULL) = (motif_revocation IS NULL))
);
CREATE INDEX session_utilisateur ON session (utilisateur_id);

-- Visibles : la session dont le jeton est présenté (authentification d'une requête, sans autre
-- contexte), ou les sessions de l'utilisateur du contexte (ses sessions, rotation, révocation,
-- purge). `sigma.jeton_session` porte l'empreinte en hexadécimal, jamais le jeton.
--
-- `sigma.utilisateur` ne doit JAMAIS désigner un utilisateur seulement revendiqué (courriel
-- saisi à la connexion, PR 8b et 8c) : les politiques `session_titulaire`,
-- `appartenance_membre`, `cabinet_membre` (ci-dessous) et `utilisateur_lecture` (0001) lui
-- ouvrent tout ce qui appartient à cet utilisateur, sessions comprises. Seul l'utilisateur
-- d'une session authentifiée, ou dont un facteur vient d'être vérifié, le pose
-- (crates/api/src/base/mod.rs, « `sigma.utilisateur` »).
ALTER TABLE session ENABLE ROW LEVEL SECURITY;
ALTER TABLE session FORCE ROW LEVEL SECURITY;
CREATE POLICY session_titulaire ON session
    FOR ALL
    USING (
        jeton_hache = decode(NULLIF(current_setting('sigma.jeton_session', true), ''), 'hex')
        OR utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
    )
    WITH CHECK (
        jeton_hache = decode(NULLIF(current_setting('sigma.jeton_session', true), ''), 'hex')
        OR utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
    );

-- Rôle applicatif seul : lecture, création, mise à jour des seules colonnes d'état (rotation,
-- activité, ré-authentification, révocation), suppression pour la purge des sessions closes de
-- l'utilisateur. Rien pour le rôle de lecture d'audit.
GRANT SELECT, INSERT, DELETE ON session TO sigma_application;
GRANT UPDATE (jeton_hache, derniere_activite, derniere_verification, revoquee_le, motif_revocation)
    ON session TO sigma_application;

-- Cabinets de l'utilisateur, **sans cabinet en contexte** (`GET /api/moi/cabinets`, choix du
-- cabinet actif) : ses appartenances (suspendues comprises : leur état est lu), et les cabinets
-- où il en a une **active** (la RLS ne laisse pas lire le nom d'un cabinet où il est suspendu ;
-- la requête et Cedar l'écartent aussi). Politiques permissives de lecture seule, liste
-- `lecture_membre` de catalogue.toml. Avec un cabinet en contexte, elles ne s'appliquent pas :
-- seules les politiques de tenant valent (F1.14).
CREATE POLICY appartenance_membre ON appartenance
    FOR SELECT
    USING (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NULL
        AND utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
    );
CREATE POLICY cabinet_membre ON cabinet
    FOR SELECT
    USING (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NULL
        AND EXISTS (
            SELECT 1 FROM appartenance a
            WHERE a.cabinet_id = cabinet.id
              AND a.utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
              AND a.actif
        )
    );
