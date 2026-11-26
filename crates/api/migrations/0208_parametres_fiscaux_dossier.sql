-- Versions de paramètres fiscaux PROPRES À UN DOSSIER (ADR 0030 point 3) : une surcharge est une
-- NOUVELLE version datée rattachée au dossier, jamais une modification. Cette migration livre la
-- table (ajout seul) et la règle qui en découle ; les routes de création et de lecture suivent
-- (ADR 0030 point 8).
--
-- Une version est un contenu complet, au même format que celui d'un paquet de pays, validé par
-- `fiscal` avant l'insertion. Son identifiant est `<pays>-<version de base>+d<numero>`.
CREATE TABLE parametres_fiscaux_dossier (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    numero integer NOT NULL CHECK (numero >= 1),
    pays text NOT NULL CHECK (pays ~ '^[A-Z]{2}$'),
    version text NOT NULL CHECK (version ~ '^[0-9]{4}\.[0-9]{1,4}$'),
    date_effet date NOT NULL,
    date_fin date CHECK (date_fin IS NULL OR date_fin >= date_effet),
    contenu text NOT NULL CHECK (length(contenu) BETWEEN 1 AND 262144),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    cree_par uuid NOT NULL REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, numero),
    UNIQUE (cabinet_id, dossier_id, date_effet),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id)
);

ALTER TABLE parametres_fiscaux_dossier ENABLE ROW LEVEL SECURITY;
ALTER TABLE parametres_fiscaux_dossier FORCE ROW LEVEL SECURITY;
CREATE POLICY parametres_fiscaux_dossier_tenant ON parametres_fiscaux_dossier
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Ajout seul : ni `UPDATE` ni `DELETE` pour le rôle applicatif.
GRANT SELECT, INSERT ON parametres_fiscaux_dossier TO sigma_application;

-- Le pays d'un dossier qui a une version propre ne change plus : la table, en ajout seul,
-- deviendrait orpheline (ADR 0030 point 3). Seconde barrière de la règle du cas d'usage ; même
-- fonction que `0206`, un cas de plus (la fonction n'est pas `SECURITY DEFINER` : elle ne voit que
-- le dossier du contexte, que le déclencheur exige déjà).
CREATE OR REPLACE FUNCTION dossier_contexte_fige() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.pays, NEW.devise_tenue, NEW.decimales_tenue)
           IS DISTINCT FROM (OLD.pays, OLD.devise_tenue, OLD.decimales_tenue) THEN
        IF NULLIF(current_setting('sigma.dossier', true), '')::uuid IS DISTINCT FROM OLD.id THEN
            RAISE EXCEPTION 'pays et devise de tenue : le contexte du dossier est requis'
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    IF (NEW.pays, NEW.devise_tenue, NEW.decimales_tenue)
           IS DISTINCT FROM (OLD.pays, OLD.devise_tenue, OLD.decimales_tenue)
       AND (
        EXISTS (SELECT 1 FROM public.ecriture_validee e
                 WHERE e.cabinet_id = OLD.cabinet_id AND e.dossier_id = OLD.id)
        OR EXISTS (SELECT 1 FROM public.brouillon b
                    WHERE b.cabinet_id = OLD.cabinet_id AND b.dossier_id = OLD.id)
    ) THEN
        RAISE EXCEPTION 'pays et devise de tenue figés : le dossier a des écritures'
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.pays IS DISTINCT FROM OLD.pays
       AND EXISTS (SELECT 1 FROM public.parametres_fiscaux_dossier p
                    WHERE p.cabinet_id = OLD.cabinet_id AND p.dossier_id = OLD.id) THEN
        RAISE EXCEPTION 'pays figé : le dossier a des versions de paramètres fiscaux propres'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
