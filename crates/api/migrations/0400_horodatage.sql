-- Horodatage de la tête de chaîne : file de tâches et jetons (L2-30, PR 15a ; ADR 0010, 0011,
-- 0014). Voie intégrité et horodatage, plage 0400 à 0499 : ce fichier ne dépend que de
-- `0001_organisation.sql` (dossier) ; il ne lit aucune table d'une autre voie.

-- File de tâches de l'instance (ADR 0010 : « tâche de la file PostgreSQL exécutée par le binaire
-- `api` »). Une seule file pour toute l'instance : un travailleur la parcourt sans connaître un
-- cabinet à l'avance. C'est pourquoi la table est une table d'**instance** : ses colonnes de
-- cible (`cabinet_cible`, `dossier_cible`) ne portent aucune clé étrangère et ne s'appellent pas
-- `cabinet_id` ni `dossier_id` (catalogue `integrite.toml`). Elle ne contient aucune donnée
-- comptable : des identifiants, des instants et des codes stables.
--
-- Natures :
-- - `horodatage` : obtenir un jeton RFC 3161 pour la tête de chaîne d'un dossier. Sujet =
--   déclencheur (`integrite::horodatage::Declencheur`) : `quotidien` (planifiée par la
--   validation d'une écriture, pour le début du jour suivant, UTC), `verrouillage`, `cloture`.
--   Une seule tâche non terminée par dossier et par sujet.
-- - `notification` : alerte ou envoi à l'administrateur du cabinet. Le courriel est celui de la
--   PR 9 (question 8) ; tant qu'il n'existe pas, la ligne attend (`en_attente`). Sujets :
--   export d'audit (#157), export d'intégrité, retard d'horodatage de plus de 24 h (F6.2),
--   témoin à envoyer au cabinet (question 9b).
CREATE TABLE tache (
    id uuid PRIMARY KEY,
    nature text NOT NULL CHECK (nature IN ('horodatage', 'notification')),
    sujet text NOT NULL,
    cabinet_cible uuid NOT NULL,
    dossier_cible uuid,
    -- Auteur de l'action qui a produit une notification (export), sans clé étrangère.
    acteur_cible uuid,
    etat text NOT NULL CHECK (etat IN ('en_attente', 'en_cours', 'terminee')),
    cree_le timestamptz NOT NULL,
    prochaine_tentative timestamptz NOT NULL,
    -- Bail d'une tâche prise : passé ce délai, un autre passage peut la reprendre (arrêt du
    -- processus pendant l'appel à l'autorité).
    reserve_jusqua timestamptz,
    tentatives integer NOT NULL DEFAULT 0 CHECK (tentatives >= 0),
    -- Code stable de la dernière issue (jamais le texte d'une réponse d'autorité).
    derniere_issue text CHECK (derniere_issue ~ '^[a-z0-9_]{1,40}$'),
    termine_le timestamptz,
    CHECK (
        (nature = 'horodatage' AND dossier_cible IS NOT NULL
            AND sujet IN ('quotidien', 'verrouillage', 'cloture'))
        OR (nature = 'notification' AND sujet IN (
            'export_audit_cabinet', 'export_audit_dossier', 'export_integrite',
            'retard_horodatage', 'temoin'))
    ),
    CHECK (sujet NOT IN ('export_audit_dossier', 'export_integrite', 'retard_horodatage',
                         'temoin') OR dossier_cible IS NOT NULL),
    CHECK (sujet <> 'export_audit_cabinet' OR dossier_cible IS NULL),
    CHECK ((etat = 'terminee') = (termine_le IS NOT NULL)),
    CHECK ((etat = 'en_cours') = (reserve_jusqua IS NOT NULL))
);
-- Une tâche d'horodatage non terminée par dossier et par déclencheur ; une seule alerte de
-- retard non traitée par dossier.
CREATE UNIQUE INDEX tache_horodatage_unique
    ON tache (cabinet_cible, dossier_cible, sujet)
    WHERE nature = 'horodatage' AND etat <> 'terminee';
CREATE UNIQUE INDEX tache_retard_unique
    ON tache (cabinet_cible, dossier_cible)
    WHERE sujet = 'retard_horodatage' AND etat <> 'terminee';
-- Reprise des tâches à échéance.
CREATE INDEX tache_echeance
    ON tache (prochaine_tentative)
    WHERE nature = 'horodatage' AND etat <> 'terminee';
-- Lecture par cabinet (état d'un dossier, alertes d'un cabinet).
CREATE INDEX tache_cible ON tache (cabinet_cible, dossier_cible, nature);

ALTER TABLE tache ENABLE ROW LEVEL SECURITY;
ALTER TABLE tache FORCE ROW LEVEL SECURITY;
-- Un tenant lit et crée les tâches de sa cible exacte : le cabinet du contexte et son dossier (ou
-- aucun dossier pour une action du cabinet). Jamais de mise à jour ni de suppression.
CREATE POLICY tache_tenant_lecture ON tache
    FOR SELECT
    USING (
        cabinet_cible = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_cible IS NOT DISTINCT FROM NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
CREATE POLICY tache_tenant_creation ON tache
    FOR INSERT
    WITH CHECK (
        cabinet_cible = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_cible IS NOT DISTINCT FROM NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Le travailleur de la file (`api::base::taches::TransactionTravailleur`) traverse les tenants :
-- sans cabinet en contexte, avec le réglage `sigma.travail`, que seule cette transaction pose.
-- Comme la connexion (`sigma.courriel_connexion`), c'est un réglage de l'application : il ne
-- remplace pas le contrôle d'accès, il borne ce que la RLS ouvre à ce code (cette seule table).
CREATE POLICY tache_travailleur ON tache
    FOR ALL
    USING (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NULL
        AND current_setting('sigma.travail', true) = 'file'
    )
    WITH CHECK (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NULL
        AND current_setting('sigma.travail', true) = 'file'
    );
GRANT SELECT, INSERT, DELETE ON tache TO sigma_application;
GRANT UPDATE (etat, prochaine_tentative, reserve_jusqua, tentatives, derniere_issue, termine_le)
    ON tache TO sigma_application;

-- Jeton d'horodatage d'un dossier : le témoin horodaté (nombre d'écritures et tête de chaîne à
-- cet instant, `integrite::horodatage::Temoin`) et le jeton RFC 3161 (DER) que l'autorité a
-- renvoyé, déjà vérifié par `integrite` à la réception. Rien d'autre n'est envoyé à l'autorité
-- (F6.3). **Ajout seul** : un jeton est une preuve, jamais modifiée ni supprimée (F2.5).
CREATE TABLE jeton_horodatage (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    nombre bigint NOT NULL CHECK (nombre >= 1),
    tete bytea NOT NULL CHECK (octet_length(tete) = 32),
    -- Nom de l'autorité (hôte et port), tel que le client la désigne.
    autorite text NOT NULL CHECK (length(autorite) BETWEEN 1 AND 255),
    -- Date attestée par le jeton, et instant où `api` l'a obtenu.
    date_jeton timestamptz NOT NULL,
    obtenu_le timestamptz NOT NULL,
    declencheur text NOT NULL CHECK (declencheur IN ('quotidien', 'verrouillage', 'cloture')),
    jeton bytea NOT NULL CHECK (octet_length(jeton) BETWEEN 1 AND 65536),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id)
);
-- Dernier jeton d'un dossier, et jetons dans l'ordre de la chaîne couverte.
CREATE INDEX jeton_horodatage_chaine
    ON jeton_horodatage (cabinet_id, dossier_id, nombre DESC, obtenu_le DESC);

ALTER TABLE jeton_horodatage ENABLE ROW LEVEL SECURITY;
ALTER TABLE jeton_horodatage FORCE ROW LEVEL SECURITY;
CREATE POLICY jeton_horodatage_tenant ON jeton_horodatage
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Ajout seul : ni UPDATE ni DELETE (catalogue `integrite.toml`).
GRANT SELECT, INSERT ON jeton_horodatage TO sigma_application;
