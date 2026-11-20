-- Méthode de secours (mot de passe et TOTP), codes de récupération, événements
-- d'authentification (ADR 0012, ADR 0017 ; L2 PR 8b, issue #170).
--
-- Exécutée par `sigma_migration` (crates/api/README.md). Règles communes : migration 0001.
-- Plage de la voie authentification (0100 à 0199) : ne dépend que des migrations du socle.
--
-- Toutes les tables ci-dessous sont des tables d'instance (un utilisateur, aucun cabinet ;
-- catalogue/facteurs.toml), sous RLS activée et forcée. Instants posés par `api` (horloge
-- injectable dans les tests) : aucune valeur par défaut.
--
-- # Connexion sans contexte : `sigma.courriel_connexion`
-- Une connexion n'a pas encore d'utilisateur authentifié : elle ne pose JAMAIS
-- `sigma.utilisateur` (migration 0004), mais `sigma.courriel_connexion`, le courriel saisi. Les
-- politiques `*_connexion` ne lui ouvrent que ce qu'il faut pour vérifier un facteur de ce
-- seul compte : sa ligne `utilisateur`, son secours et ses codes (lecture, et consommation du
-- pas TOTP ou d'un code), l'inscription d'un événement. Ni ses sessions, ni ses cabinets, ni
-- la création d'un facteur. Aucune fonction SECURITY DEFINER (note d'étude de la PR 8, §2).

-- Utilisateur dont le courriel est saisi à la connexion (comparaison sans casse, comme l'index
-- unique `utilisateur_courriel_unique`).
CREATE POLICY utilisateur_connexion ON utilisateur
    FOR SELECT
    USING (lower(courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), '')));

-- Méthode de secours : mot de passe (chaîne PHC Argon2id) et secret TOTP chiffré par la clé de
-- l'instance (AES-256-GCM : nonce, chiffré, étiquette ; identifiant de la clé). Toujours
-- ensemble (ADR 0012). `totp_dernier_pas` : dernier pas accepté, base de l'anti-rejeu (un code
-- n'est admis que si son pas le dépasse, par une mise à jour conditionnelle).
CREATE TABLE facteur_secours (
    utilisateur_id uuid PRIMARY KEY REFERENCES utilisateur (id),
    mot_de_passe text NOT NULL
        CHECK (mot_de_passe LIKE '$argon2id$%' AND length(mot_de_passe) <= 200),
    totp_chiffre bytea NOT NULL CHECK (length(totp_chiffre) = 48),
    totp_cle_id bytea NOT NULL CHECK (length(totp_cle_id) = 8),
    totp_dernier_pas bigint NOT NULL CHECK (totp_dernier_pas >= 0),
    cree_le timestamptz NOT NULL,
    modifie_le timestamptz NOT NULL
);

-- Codes de récupération : seule leur empreinte SHA-256 est stockée. Usage unique : `utilise_le`
-- posé par une mise à jour conditionnelle (`utilise_le IS NULL`), atomique.
CREATE TABLE code_recuperation (
    id uuid PRIMARY KEY,
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    empreinte bytea NOT NULL UNIQUE CHECK (length(empreinte) = 32),
    cree_le timestamptz NOT NULL,
    utilise_le timestamptz
);
CREATE INDEX code_recuperation_utilisateur ON code_recuperation (utilisateur_id);

-- Journal d'instance des événements d'authentification (ADR 0017) : en ajout seul, non chaîné.
-- Rien pour un courriel inconnu (utilisateur obligatoire). Jamais de mot de passe, de code, de
-- jeton ni d'empreinte. `session_id` sans clé étrangère : la session est purgée après 30 jours,
-- l'événement reste.
CREATE TABLE evenement_authentification (
    id uuid PRIMARY KEY,
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    session_id uuid,
    nature text NOT NULL CHECK (nature IN (
        'connexion', 'reauthentification', 'enrolement_secours', 'generation_codes',
        'deconnexion', 'revocation_session'
    )),
    methode text CHECK (methode IN ('passkey', 'secours', 'code_recuperation', 'lien_enrolement')),
    issue text NOT NULL CHECK (issue IN ('succes', 'echec')),
    instant timestamptz NOT NULL,
    appareil text CHECK (length(appareil) <= 200),
    adresse inet
);
CREATE INDEX evenement_authentification_utilisateur
    ON evenement_authentification (utilisateur_id, instant DESC);

-- RLS : le titulaire (utilisateur authentifié, `sigma.utilisateur`) ; à la connexion, le
-- compte du courriel saisi, pour les seules commandes nécessaires.
ALTER TABLE facteur_secours ENABLE ROW LEVEL SECURITY;
ALTER TABLE facteur_secours FORCE ROW LEVEL SECURITY;
CREATE POLICY facteur_secours_titulaire ON facteur_secours
    FOR ALL
    USING (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid)
    WITH CHECK (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);
CREATE POLICY facteur_secours_connexion ON facteur_secours
    FOR SELECT
    USING (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ));
CREATE POLICY facteur_secours_connexion_pas ON facteur_secours
    FOR UPDATE
    USING (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ))
    WITH CHECK (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ));

ALTER TABLE code_recuperation ENABLE ROW LEVEL SECURITY;
ALTER TABLE code_recuperation FORCE ROW LEVEL SECURITY;
CREATE POLICY code_recuperation_titulaire ON code_recuperation
    FOR ALL
    USING (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid)
    WITH CHECK (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);
CREATE POLICY code_recuperation_connexion ON code_recuperation
    FOR SELECT
    USING (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ));
CREATE POLICY code_recuperation_connexion_usage ON code_recuperation
    FOR UPDATE
    USING (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ))
    WITH CHECK (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ));

-- Ce que la connexion peut modifier, colonne par colonne : une politique ne compare pas l'avant
-- et l'après d'une ligne, un déclencheur `BEFORE UPDATE` le fait. Sans utilisateur authentifié
-- (`sigma.utilisateur` vide), seules passent la consommation d'un pas TOTP (`totp_dernier_pas`
-- en hausse stricte, rien d'autre) et l'usage d'un code (`utilise_le` de NULL à une date, rien
-- d'autre) ; toute autre mise à jour est refusée (SQLSTATE 42501). Avec un utilisateur
-- authentifié, ses propres lignes restent modifiables (enrôlement, re-hachage). Fonctions sans
-- `SECURITY DEFINER` (droits de l'appelant), `EXECUTE` retiré à PUBLIC, `search_path` figé.
CREATE FUNCTION facteur_secours_garde_connexion() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NULLIF(current_setting('sigma.utilisateur', true), '') IS NULL
       AND (ROW(NEW.utilisateur_id, NEW.mot_de_passe, NEW.totp_chiffre, NEW.totp_cle_id,
                NEW.cree_le, NEW.modifie_le)
                IS DISTINCT FROM
            ROW(OLD.utilisateur_id, OLD.mot_de_passe, OLD.totp_chiffre, OLD.totp_cle_id,
                OLD.cree_le, OLD.modifie_le)
            OR NEW.totp_dernier_pas <= OLD.totp_dernier_pas) THEN
        RAISE EXCEPTION 'connexion : seule la consommation d''un pas TOTP est admise'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION facteur_secours_garde_connexion() FROM PUBLIC;
CREATE TRIGGER facteur_secours_garde_connexion
    BEFORE UPDATE ON facteur_secours
    FOR EACH ROW EXECUTE FUNCTION facteur_secours_garde_connexion();

CREATE FUNCTION code_recuperation_garde_connexion() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NULLIF(current_setting('sigma.utilisateur', true), '') IS NULL
       AND (ROW(NEW.id, NEW.utilisateur_id, NEW.empreinte, NEW.cree_le)
                IS DISTINCT FROM ROW(OLD.id, OLD.utilisateur_id, OLD.empreinte, OLD.cree_le)
            OR OLD.utilise_le IS NOT NULL OR NEW.utilise_le IS NULL) THEN
        RAISE EXCEPTION 'connexion : seul l''usage d''un code inutilisé est admis'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION code_recuperation_garde_connexion() FROM PUBLIC;
CREATE TRIGGER code_recuperation_garde_connexion
    BEFORE UPDATE ON code_recuperation
    FOR EACH ROW EXECUTE FUNCTION code_recuperation_garde_connexion();

-- Événements : lus par leur seul titulaire ; inscrits par lui. À la connexion, pour le compte
-- du courriel saisi, seulement l'**échec** d'une connexion, sans session (la réussite est
-- inscrite après la vérification du facteur, par l'utilisateur authentifié), et la lecture de
-- ces seuls échecs (plafond d'échecs par compte).
ALTER TABLE evenement_authentification ENABLE ROW LEVEL SECURITY;
ALTER TABLE evenement_authentification FORCE ROW LEVEL SECURITY;
CREATE POLICY evenement_titulaire ON evenement_authentification
    FOR ALL
    USING (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid)
    WITH CHECK (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);
CREATE POLICY evenement_connexion ON evenement_authentification
    FOR INSERT
    WITH CHECK (
        nature = 'connexion' AND issue = 'echec' AND session_id IS NULL
        AND utilisateur_id IN (
            SELECT u.id FROM utilisateur u
            WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
        )
    );
CREATE POLICY evenement_connexion_echecs ON evenement_authentification
    FOR SELECT
    USING (
        nature = 'connexion' AND issue = 'echec'
        AND utilisateur_id IN (
            SELECT u.id FROM utilisateur u
            WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
        )
    );

-- Droits du rôle applicatif seul ; rien pour le rôle de lecture d'audit.
-- Secours : création et remplacement (enrôlement), consommation du pas TOTP, re-hachage.
GRANT SELECT, INSERT ON facteur_secours TO sigma_application;
GRANT UPDATE (mot_de_passe, totp_chiffre, totp_cle_id, totp_dernier_pas, modifie_le)
    ON facteur_secours TO sigma_application;
-- Codes : nouvelle série (suppression de l'ancienne), usage.
GRANT SELECT, INSERT, DELETE ON code_recuperation TO sigma_application;
GRANT UPDATE (utilise_le) ON code_recuperation TO sigma_application;
-- Événements : ajout seul (ni UPDATE ni DELETE ; liste `ajout_seul` du catalogue).
GRANT SELECT, INSERT ON evenement_authentification TO sigma_application;

-- Un changement de facteur révoque les autres sessions de l'utilisateur (ADR 0012).
ALTER TABLE session DROP CONSTRAINT session_motif_revocation_check;
ALTER TABLE session ADD CONSTRAINT session_motif_revocation_check
    CHECK (motif_revocation IN ('deconnexion', 'revocation_utilisateur', 'changement_facteur'));
