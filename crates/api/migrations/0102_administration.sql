-- Administration du cabinet (L2-19, PR 9a ; ADR 0012, 0014, 0017, 0023).
--
-- Voie authentification, plage 0100 à 0199. Dépend de `0001` (cabinet, appartenance), `0004`
-- (session) et `0100` (secours) : même plage ou socle, aucune dépendance vers une autre voie.
--
-- Contenu : suspension d'une appartenance, gestion des sessions d'un membre par
-- l'administrateur, invitations, réinitialisation d'un compte par un administrateur, et la table
-- des liens à usage unique (invitation, réinitialisation).

-- Suspension (ADR 0014) : la colonne `actif` existe depuis `0002` ; le rôle applicatif peut
-- désormais la modifier (le rôle l'était déjà). Aucune suppression d'appartenance ne change.
GRANT UPDATE (actif) ON appartenance TO sigma_application;

-- Retrait de la méthode de secours d'un utilisateur dont l'accès est réinitialisé (ADR 0012,
-- « les sessions, passkeys, secours et codes de l'utilisateur sont révoqués »).
GRANT DELETE ON facteur_secours TO sigma_application;

-- Motifs de révocation d'une session : ceux de l'administration s'ajoutent.
ALTER TABLE session DROP CONSTRAINT session_motif_revocation_check;
ALTER TABLE session ADD CONSTRAINT session_motif_revocation_check
    CHECK (motif_revocation IN (
        'deconnexion', 'revocation_utilisateur', 'changement_facteur',
        'revocation_administrateur', 'changement_droits', 'reinitialisation'
    ));

-- Sessions d'un membre du cabinet du contexte, vues et révoquées par un **administrateur actif**
-- de ce cabinet (spec §21 : « l'utilisateur et l'administrateur voient les sessions actives et
-- les révoquent »). Le titulaire de la session doit être membre du cabinet du contexte (la
-- sous-requête lit `appartenance`, sous RLS : limitée à ce cabinet) et l'utilisateur du contexte
-- doit en être administrateur actif, lu en base. Cedar décide aussi ; ceci est la défense en
-- profondeur. Politiques permissives de plus sur `session` : sans cabinet en contexte, elles ne
-- voient rien (une sous-requête sur `appartenance` n'y voit que les lignes de l'utilisateur,
-- jamais le rôle d'administrateur d'un autre cabinet que celui du contexte).
CREATE POLICY session_administration_lecture ON session
    FOR SELECT
    USING (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NOT NULL
        AND EXISTS (SELECT 1 FROM appartenance m WHERE m.utilisateur_id = session.utilisateur_id)
        AND EXISTS (
            SELECT 1 FROM appartenance adm
            WHERE adm.utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
              AND adm.role = 'administrateur' AND adm.actif
        )
    );
CREATE POLICY session_administration_revocation ON session
    FOR UPDATE
    USING (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NOT NULL
        AND EXISTS (SELECT 1 FROM appartenance m WHERE m.utilisateur_id = session.utilisateur_id)
        AND EXISTS (
            SELECT 1 FROM appartenance adm
            WHERE adm.utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
              AND adm.role = 'administrateur' AND adm.actif
        )
    )
    WITH CHECK (
        NULLIF(current_setting('sigma.cabinet', true), '') IS NOT NULL
        AND EXISTS (SELECT 1 FROM appartenance m WHERE m.utilisateur_id = session.utilisateur_id)
        AND EXISTS (
            SELECT 1 FROM appartenance adm
            WHERE adm.utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid
              AND adm.role = 'administrateur' AND adm.actif
        )
    );

-- Invitation à rejoindre un cabinet avec un rôle (ADR 0012). Le lien d'invitation n'est pas ici :
-- il est dans `lien_acces`, haché, émis par la file de tâches (PR 9b). Une seule invitation en
-- attente par courriel et par cabinet. Le courriel n'est jamais affiché hors du cabinet.
CREATE TABLE invitation (
    cabinet_id uuid NOT NULL REFERENCES cabinet (id),
    id uuid NOT NULL,
    courriel text NOT NULL CHECK (length(courriel) BETWEEN 3 AND 254 AND position('@' IN courriel) > 1),
    nom text NOT NULL CHECK (length(btrim(nom)) BETWEEN 1 AND 200),
    role text NOT NULL CHECK (role IN ('administrateur', 'valideur', 'collaborateur', 'auditeur')),
    -- Administrateur qui invite ; NULL pour la commande d'administration de l'instance
    -- (première invitation d'un cabinet).
    invite_par uuid,
    cree_le timestamptz NOT NULL,
    etat text NOT NULL CHECK (etat IN ('en_attente', 'acceptee', 'revoquee')),
    traitee_le timestamptz,
    -- Compte lié à l'acceptation.
    utilisateur_id uuid,
    PRIMARY KEY (cabinet_id, id),
    UNIQUE (id),
    CHECK ((etat = 'en_attente') = (traitee_le IS NULL)),
    CHECK ((etat = 'acceptee') = (utilisateur_id IS NOT NULL))
);
CREATE UNIQUE INDEX invitation_en_attente_unique
    ON invitation (cabinet_id, lower(courriel)) WHERE etat = 'en_attente';

ALTER TABLE invitation ENABLE ROW LEVEL SECURITY;
ALTER TABLE invitation FORCE ROW LEVEL SECURITY;
CREATE POLICY invitation_tenant ON invitation
    FOR ALL
    USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);
GRANT SELECT, INSERT ON invitation TO sigma_application;
GRANT UPDATE (etat, traitee_le, utilisateur_id) ON invitation TO sigma_application;

-- Réinitialisation de l'accès d'un membre par un administrateur (ADR 0012, F1.12) : vérification
-- hors bande déclarée, motif, délai de 24 heures pendant lequel l'utilisateur et les autres
-- administrateurs peuvent annuler ; au terme du délai, un lien d'enrôlement (1 heure) est émis.
-- Un administrateur ne réinitialise jamais son propre compte (contrôlé par la requête).
CREATE TABLE reinitialisation (
    cabinet_id uuid NOT NULL REFERENCES cabinet (id),
    id uuid NOT NULL,
    utilisateur_id uuid NOT NULL,
    -- Administrateur demandeur ; NULL pour la procédure de support de l'instance (commande).
    demandee_par uuid,
    -- Code du motif (liste fermée) : seul inscrit à l'audit chaîné, qui est définitif. Le texte
    -- libre `motif` reste ici, dans une table dont les lignes peuvent être effacées (loi 2008-12).
    motif_code text NOT NULL CHECK (motif_code IN (
        'appareil_perdu', 'facteurs_perdus', 'compte_compromis', 'depart_reprise', 'autre')),
    motif text NOT NULL CHECK (length(btrim(motif)) BETWEEN 1 AND 500),
    verification_hors_bande boolean NOT NULL CHECK (verification_hors_bande),
    cree_le timestamptz NOT NULL,
    echeance timestamptz NOT NULL,
    etat text NOT NULL CHECK (etat IN ('en_attente', 'annulee', 'aboutie', 'expiree')),
    traitee_le timestamptz,
    PRIMARY KEY (cabinet_id, id),
    UNIQUE (id),
    FOREIGN KEY (cabinet_id, utilisateur_id) REFERENCES appartenance (cabinet_id, utilisateur_id),
    CHECK (echeance > cree_le),
    CHECK ((etat = 'en_attente') = (traitee_le IS NULL))
);
CREATE UNIQUE INDEX reinitialisation_en_cours
    ON reinitialisation (cabinet_id, utilisateur_id) WHERE etat = 'en_attente';

ALTER TABLE reinitialisation ENABLE ROW LEVEL SECURITY;
ALTER TABLE reinitialisation FORCE ROW LEVEL SECURITY;
CREATE POLICY reinitialisation_tenant ON reinitialisation
    FOR ALL
    USING (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);
GRANT SELECT, INSERT ON reinitialisation TO sigma_application;
GRANT UPDATE (etat, traitee_le) ON reinitialisation TO sigma_application;

-- Liens à usage unique (invitation, réinitialisation ; ADR 0012) : 256 bits d'aléa, **seule leur
-- empreinte** SHA-256 est stockée (jamais le lien : une lecture de la base ne suffit pas à
-- prendre un compte). Table d'instance : le porteur d'un lien n'a ni session ni cabinet en
-- contexte ; elle ne porte ni `cabinet_id` ni `dossier_id` (cibles sans clé étrangère).
-- Un seul lien vivant par cible : en émettre un neuf ferme le précédent.
CREATE TABLE lien_acces (
    id uuid PRIMARY KEY,
    nature text NOT NULL CHECK (nature IN ('invitation', 'reinitialisation')),
    cabinet_cible uuid NOT NULL,
    -- Identifiant de l'invitation ou de la réinitialisation.
    cible uuid NOT NULL,
    jeton_hache bytea NOT NULL UNIQUE CHECK (length(jeton_hache) = 32),
    cree_le timestamptz NOT NULL,
    expire_le timestamptz NOT NULL CHECK (expire_le > cree_le),
    ferme_le timestamptz,
    -- Raison de la fermeture : utilisé, remplacé par un lien neuf, ou annulé avec sa cible.
    raison text CHECK (raison IN ('utilise', 'remplace', 'annule')),
    CHECK ((ferme_le IS NULL) = (raison IS NULL))
);
CREATE INDEX lien_acces_cible ON lien_acces (cabinet_cible, nature, cible);

ALTER TABLE lien_acces ENABLE ROW LEVEL SECURITY;
ALTER TABLE lien_acces FORCE ROW LEVEL SECURITY;
-- Le cabinet émet, remplace et annule les liens de ses invitations et réinitialisations.
CREATE POLICY lien_acces_tenant ON lien_acces
    FOR ALL
    USING (cabinet_cible = NULLIF(current_setting('sigma.cabinet', true), '')::uuid)
    WITH CHECK (cabinet_cible = NULLIF(current_setting('sigma.cabinet', true), '')::uuid);
-- Le porteur d'un lien le présente : seule la ligne de son empreinte est visible, sans cabinet ni
-- utilisateur en contexte (`sigma.jeton_lien`, empreinte en hexadécimal, jamais le lien).
CREATE POLICY lien_acces_porteur ON lien_acces
    FOR ALL
    USING (jeton_hache = decode(NULLIF(current_setting('sigma.jeton_lien', true), ''), 'hex'))
    WITH CHECK (jeton_hache = decode(NULLIF(current_setting('sigma.jeton_lien', true), ''), 'hex'));
GRANT SELECT, INSERT ON lien_acces TO sigma_application;
GRANT UPDATE (ferme_le, raison) ON lien_acces TO sigma_application;
