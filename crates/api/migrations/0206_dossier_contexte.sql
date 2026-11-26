-- Contexte du dossier (ADR 0027) : pays, devise de tenue et échelle décimale, fuseau, langue,
-- forme juridique ; identifiants légaux génériques ; devise d'une ligne comparée à la devise de
-- tenue du dossier (au lieu du XOF figé) ; format canonique v4 (devise de tenue dans le contenu
-- haché, `docs/integrite/format-v5.md`).
--
-- Numéro `0206` dans la plage de la voie comptabilité. Ce fichier dépend de `0001` (dossier),
-- `0202` et `0203` (lignes), toutes de numéro inférieur et fusionnées.
--
-- Dossiers existants : XOF, sans décimale, Sénégal, Africa/Dakar, français (tout dossier créé
-- jusqu'ici l'était en XOF, spec §5.5). Les valeurs par défaut ne servent qu'à cette
-- reprise : elles sont retirées aussitôt, un nouveau dossier doit dire son pays et sa devise
-- (la devise de tenue est immuable : un défaut silencieux la fixerait à tort).

ALTER TABLE dossier
    ADD COLUMN pays text NOT NULL DEFAULT 'SN' CHECK (pays ~ '^[A-Z]{2}$'),
    ADD COLUMN devise_tenue text NOT NULL DEFAULT 'XOF' CHECK (devise_tenue ~ '^[A-Z]{3}$'),
    ADD COLUMN decimales_tenue smallint NOT NULL DEFAULT 0
        CHECK (decimales_tenue BETWEEN 0 AND 4),
    ADD COLUMN fuseau text NOT NULL DEFAULT 'Africa/Dakar'
        CHECK (length(fuseau) <= 64 AND fuseau ~ '^[A-Za-z0-9_+-]+(/[A-Za-z0-9_+-]+)+$'),
    ADD COLUMN langue text NOT NULL DEFAULT 'fr'
        CHECK (length(langue) <= 35 AND langue ~ '^[a-z]{2,3}(-[A-Za-z0-9]{1,8})*$'),
    ADD COLUMN forme_juridique text
        CHECK (forme_juridique IS NULL
               OR (length(btrim(forme_juridique)) BETWEEN 1 AND 100
                   AND forme_juridique !~ '[[:cntrl:]]'));

ALTER TABLE dossier
    ALTER COLUMN pays DROP DEFAULT,
    ALTER COLUMN devise_tenue DROP DEFAULT,
    ALTER COLUMN decimales_tenue DROP DEFAULT,
    ALTER COLUMN fuseau DROP DEFAULT,
    ALTER COLUMN langue DROP DEFAULT;

-- Le rôle applicatif ne reçoit toujours que `UPDATE (nom)` sur `dossier` (migration `0001`) :
-- pays, devise de tenue, échelle, fuseau, langue et forme juridique sont fixés à la création.
-- La devise de tenue est dans le contenu haché de chaque écriture (format v4) ; elle ne change
-- plus après la création (ADR 0027 point 1).

-- Identifiants légaux du dossier (NINEA, RCCM, NIF…) : couples type/valeur. Le type est un code
-- du pack pays (`crates/syscohada/data/pays.toml`), contrôlé par l'API ; la valeur est un texte
-- libre borné (aucun format national n'est sourcé). Table de dossier à la règle commune (RLS
-- forcée, cabinet et dossier du contexte), en ajout seul : créés avec le dossier, aucune
-- modification ni suppression exposée (catalogue `organisation.toml`).
CREATE TABLE dossier_identifiant_legal (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    type text NOT NULL CHECK (type ~ '^[A-Z][A-Z0-9_]{1,31}$'),
    valeur text NOT NULL CHECK (
        length(valeur) <= 64 AND length(btrim(valeur)) >= 1 AND valeur !~ '[[:cntrl:]]'
    ),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, type),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id)
);

ALTER TABLE dossier_identifiant_legal ENABLE ROW LEVEL SECURITY;
ALTER TABLE dossier_identifiant_legal FORCE ROW LEVEL SECURITY;
CREATE POLICY dossier_identifiant_legal_tenant ON dossier_identifiant_legal
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
GRANT SELECT, INSERT ON dossier_identifiant_legal TO sigma_application;

-- Devise d'une ligne en devise : un code ISO 4217 **autre que la devise de tenue du dossier**
-- (au lieu du XOF figé des migrations `0202` et `0203`). Une contrainte de colonne ne lit pas
-- une autre table : un déclencheur compare à `dossier.devise_tenue`. La forme du code reste une
-- contrainte de colonne.
ALTER TABLE brouillon_ligne
    DROP CONSTRAINT brouillon_ligne_devise_check,
    ADD CONSTRAINT brouillon_ligne_devise_forme CHECK (devise ~ '^[A-Z]{3}$');
ALTER TABLE ecriture_validee_ligne
    DROP CONSTRAINT ecriture_validee_ligne_devise_check,
    ADD CONSTRAINT ecriture_validee_ligne_devise_forme CHECK (devise ~ '^[A-Z]{3}$');

-- Fonction ordinaire (sans `SECURITY DEFINER`), `EXECUTE` retiré à `PUBLIC`, chemin de
-- recherche fixé : le rôle applicatif la déclenche sous sa propre RLS, qui lui montre son dossier.
CREATE FUNCTION ligne_devise_autre_que_tenue() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NEW.devise IS NOT NULL AND NEW.devise = (
        SELECT d.devise_tenue FROM public.dossier d
         WHERE d.cabinet_id = NEW.cabinet_id AND d.id = NEW.dossier_id
    ) THEN
        RAISE EXCEPTION 'devise de la ligne égale à la devise de tenue du dossier'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION ligne_devise_autre_que_tenue() FROM PUBLIC;

CREATE TRIGGER brouillon_ligne_devise_autre_que_tenue
    BEFORE INSERT OR UPDATE ON brouillon_ligne
    FOR EACH ROW EXECUTE FUNCTION ligne_devise_autre_que_tenue();
CREATE TRIGGER ecriture_validee_ligne_devise_autre_que_tenue
    BEFORE INSERT ON ecriture_validee_ligne
    FOR EACH ROW EXECUTE FUNCTION ligne_devise_autre_que_tenue();

-- Format canonique de l'écriture validée : 4 = devise de tenue et échelle dans le contenu haché.
ALTER TABLE ecriture_validee
    DROP CONSTRAINT ecriture_validee_format_check,
    ADD CONSTRAINT ecriture_validee_format_check CHECK (format IN (1, 2, 3, 4));
