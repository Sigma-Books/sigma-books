-- Passkeys, WebAuthn (ADR 0006, ADR 0012, ADR 0018 ; L2 PR 8c).
--
-- Exécutée par `sigma_migration` (crates/api/README.md). Règles communes : migration 0001.
-- Plage de la voie authentification (0100 à 0199) : ne dépend que du socle et de la 0100
-- (`evenement_authentification`), de la même voie.
--
-- Deux tables d'instance (un utilisateur, aucun cabinet ; catalogue/passkeys.toml), sous RLS
-- activée et forcée, comme celles de la 0100 :
-- - `facteur_passkey` : les passkeys d'un utilisateur. `cle` est la `Passkey` de `webauthn-rs`
--   sérialisée (clé **publique**, identifiant, compteur de signatures, indicateurs de
--   sauvegarde) : aucun secret ne vit chez nous, la clé privée reste dans l'authentificateur ;
-- - `defi_webauthn` : l'état d'une cérémonie en cours (défi d'enrôlement, de connexion ou de
--   ré-authentification), côté serveur seulement. Il est à usage unique (supprimé quand il
--   sert), échoit au bout de 5 minutes, et n'est retrouvé que par l'empreinte SHA-256 du jeton
--   du cookie `__Host-sigma_ceremonie` (jamais le jeton lui-même).
--
-- Connexion sans contexte : comme la 0100, la connexion ne pose JAMAIS `sigma.utilisateur` mais
-- `sigma.courriel_connexion` ; l'état du défi n'est lisible que sur présentation de l'empreinte
-- du jeton (`sigma.jeton_defi`), pour le seul compte du courriel saisi.

-- Passkeys d'un utilisateur. `identifiant_credential` est unique dans toute l'instance
-- (WebAuthn : un identifiant ne s'enregistre pour aucun autre compte). `cle` : JSON de la
-- `Passkey`, mis à jour à chaque connexion (compteur). `nom` : libellé choisi par l'utilisateur.
CREATE TABLE facteur_passkey (
    id uuid PRIMARY KEY,
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    identifiant_credential bytea NOT NULL UNIQUE
        CHECK (length(identifiant_credential) BETWEEN 16 AND 1023),
    cle jsonb NOT NULL CHECK (jsonb_typeof(cle) = 'object' AND length(cle::text) <= 8192),
    nom text NOT NULL CHECK (length(nom) BETWEEN 1 AND 100),
    cree_le timestamptz NOT NULL,
    derniere_utilisation timestamptz
);
CREATE INDEX facteur_passkey_utilisateur ON facteur_passkey (utilisateur_id);

-- État d'une cérémonie WebAuthn en cours. `session_id` : pour l'enrôlement et la
-- ré-authentification, la session de la requête (un défi ne se termine pas depuis une autre) ;
-- vide pour une connexion.
CREATE TABLE defi_webauthn (
    id uuid PRIMARY KEY,
    jeton_hache bytea NOT NULL UNIQUE CHECK (length(jeton_hache) = 32),
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    nature text NOT NULL CHECK (nature IN ('connexion', 'reauthentification', 'enrolement')),
    session_id uuid REFERENCES session (id) ON DELETE CASCADE,
    etat jsonb NOT NULL CHECK (jsonb_typeof(etat) = 'object' AND length(etat::text) <= 16384),
    cree_le timestamptz NOT NULL,
    expire_le timestamptz NOT NULL CHECK (expire_le > cree_le),
    CHECK ((nature = 'connexion') = (session_id IS NULL))
);
CREATE INDEX defi_webauthn_utilisateur ON defi_webauthn (utilisateur_id, nature, cree_le);

-- Événements : les passkeys ajoutent l'enrôlement et la suppression d'une passkey.
ALTER TABLE evenement_authentification DROP CONSTRAINT evenement_authentification_nature_check;
ALTER TABLE evenement_authentification ADD CONSTRAINT evenement_authentification_nature_check
    CHECK (nature IN (
        'connexion', 'reauthentification', 'enrolement_secours', 'generation_codes',
        'deconnexion', 'revocation_session', 'enrolement_passkey', 'suppression_passkey'
    ));

-- RLS : le titulaire (utilisateur authentifié, `sigma.utilisateur`) ; à la connexion, le compte
-- du courriel saisi, pour les seules commandes nécessaires.
ALTER TABLE facteur_passkey ENABLE ROW LEVEL SECURITY;
ALTER TABLE facteur_passkey FORCE ROW LEVEL SECURITY;
CREATE POLICY facteur_passkey_titulaire ON facteur_passkey
    FOR ALL
    USING (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid)
    WITH CHECK (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);
CREATE POLICY facteur_passkey_connexion ON facteur_passkey
    FOR SELECT
    USING (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ));
CREATE POLICY facteur_passkey_connexion_compteur ON facteur_passkey
    FOR UPDATE
    USING (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ))
    WITH CHECK (utilisateur_id IN (
        SELECT u.id FROM utilisateur u
        WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
    ));

ALTER TABLE defi_webauthn ENABLE ROW LEVEL SECURITY;
ALTER TABLE defi_webauthn FORCE ROW LEVEL SECURITY;
CREATE POLICY defi_webauthn_titulaire ON defi_webauthn
    FOR ALL
    USING (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid)
    WITH CHECK (utilisateur_id = NULLIF(current_setting('sigma.utilisateur', true), '')::uuid);
-- Connexion : créer un défi pour le compte du courriel saisi (nature `connexion` seulement) ;
-- retrouver et consommer le sien, sur présentation de l'empreinte de son jeton ; élaguer ses
-- anciens défis.
CREATE POLICY defi_webauthn_connexion_creation ON defi_webauthn
    FOR INSERT
    WITH CHECK (
        nature = 'connexion' AND session_id IS NULL
        AND utilisateur_id IN (
            SELECT u.id FROM utilisateur u
            WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
        )
    );
CREATE POLICY defi_webauthn_connexion_lecture ON defi_webauthn
    FOR SELECT
    USING (
        nature = 'connexion'
        AND jeton_hache = decode(NULLIF(current_setting('sigma.jeton_defi', true), ''), 'hex')
        AND utilisateur_id IN (
            SELECT u.id FROM utilisateur u
            WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
        )
    );
CREATE POLICY defi_webauthn_connexion_suppression ON defi_webauthn
    FOR DELETE
    USING (
        nature = 'connexion'
        AND utilisateur_id IN (
            SELECT u.id FROM utilisateur u
            WHERE lower(u.courriel) = lower(NULLIF(current_setting('sigma.courriel_connexion', true), ''))
        )
    );

-- Ce que la connexion peut modifier d'une passkey : son état (`cle`, compteur) et sa dernière
-- utilisation, rien d'autre (même méthode que la 0100). Avec un utilisateur authentifié, ses
-- propres lignes restent modifiables. Sans `SECURITY DEFINER`, `EXECUTE` retiré à PUBLIC,
-- `search_path` figé.
CREATE FUNCTION facteur_passkey_garde_connexion() RETURNS trigger
    LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF NULLIF(current_setting('sigma.utilisateur', true), '') IS NULL
       AND ROW(NEW.id, NEW.utilisateur_id, NEW.identifiant_credential, NEW.nom, NEW.cree_le)
               IS DISTINCT FROM
           ROW(OLD.id, OLD.utilisateur_id, OLD.identifiant_credential, OLD.nom, OLD.cree_le) THEN
        RAISE EXCEPTION 'connexion : seuls l''état et la dernière utilisation d''une passkey changent'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION facteur_passkey_garde_connexion() FROM PUBLIC;
CREATE TRIGGER facteur_passkey_garde_connexion
    BEFORE UPDATE ON facteur_passkey
    FOR EACH ROW EXECUTE FUNCTION facteur_passkey_garde_connexion();

-- Droits du rôle applicatif seul ; rien pour le rôle de lecture d'audit.
GRANT SELECT, INSERT, DELETE ON facteur_passkey TO sigma_application;
GRANT UPDATE (cle, derniere_utilisation) ON facteur_passkey TO sigma_application;
GRANT SELECT, INSERT, DELETE ON defi_webauthn TO sigma_application;
