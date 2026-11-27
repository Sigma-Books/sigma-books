-- Comptes de trésorerie (L3-21, PR 11 de `docs/lots/L3.md` ; ADR 0034 points 1 à 3 ; spec §9.1).
--
-- Un compte de trésorerie = un compte du plan + un journal de trésorerie (consensus Odoo,
-- `account_journal.py:918-952` ; Tryton, journal de relevé portant un compte,
-- `account_statement/journal.py:53-59`). Le sous-compte et le journal sont créés par la création
-- assistée (`api`, `muter` du référentiel), ou un journal de trésorerie existant est repris
-- (journal CA par défaut). Aucun numéro de compte ici : le compte par défaut d'un type est une
-- marque de `crates/syscohada/data/comptes.toml` (règle absolue 9).
--
-- Configuration (régime « C » du plan) : compte, journal, type et devise ne changent jamais (aucun
-- `UPDATE` sur ces colonnes) ; le nom, le plafond de caisse facultatif, les comptes d'écart de
-- caisse désignés et l'archivage sont modifiables et journalisés. Le plafond n'a **aucune valeur
-- par défaut** (ADR 0034 point 2 : aucun plafond défini par les textes).
CREATE TABLE compte_tresorerie (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    type text NOT NULL CHECK (type IN ('banque', 'caisse', 'portefeuille')),
    nom text NOT NULL CHECK (
        length(nom) <= 200 AND length(btrim(nom)) >= 1 AND nom !~ '[[:cntrl:]]'
    ),
    compte_id uuid NOT NULL,
    journal_id uuid NOT NULL,
    -- En L3, la devise de tenue du dossier (ADR 0034 point 1) : contrôlé par déclencheur.
    devise text NOT NULL CHECK (devise ~ '^[A-Z]{3}$'),
    plafond numeric CHECK (plafond > 0 AND plafond < 100000000000000000000),
    -- Comptes d'écart de l'arrêté de caisse (ADR 0034 point 3 ; Odoo `profit_account_id` et
    -- `loss_account_id` par journal, `account_journal.py:234-243`) : excédent (comptage supérieur
    -- au solde) et manquant. Désignés par le dossier, aucun par défaut.
    compte_excedent_id uuid,
    compte_manquant_id uuid,
    archive boolean NOT NULL DEFAULT false,
    cree_par uuid NOT NULL REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_id) REFERENCES compte (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, journal_id) REFERENCES journal (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_excedent_id)
        REFERENCES compte (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_manquant_id)
        REFERENCES compte (cabinet_id, dossier_id, id),
    -- Un compte et un journal ne portent qu'un compte de trésorerie.
    UNIQUE (cabinet_id, dossier_id, compte_id),
    UNIQUE (cabinet_id, dossier_id, journal_id),
    -- Plafond et comptes d'écart : propres à une caisse.
    CHECK (type = 'caisse' OR (plafond IS NULL AND compte_excedent_id IS NULL
                               AND compte_manquant_id IS NULL))
);

ALTER TABLE compte_tresorerie ENABLE ROW LEVEL SECURITY;
ALTER TABLE compte_tresorerie FORCE ROW LEVEL SECURITY;
CREATE POLICY compte_tresorerie_tenant ON compte_tresorerie
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT ON compte_tresorerie TO sigma_application;
GRANT UPDATE (nom, plafond, compte_excedent_id, compte_manquant_id, archive)
    ON compte_tresorerie TO sigma_application;

-- La devise d'un compte de trésorerie est celle de tenue du dossier (ADR 0034 point 1 : un compte
-- en devise étrangère relève de L6). Seconde barrière du contrôle de `api` (422
-- `tresorerie_devise_etrangere`). Fonction ordinaire (droits de l'appelant, sous RLS).
CREATE FUNCTION compte_tresorerie_devise_de_tenue() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.dossier d
                    WHERE d.cabinet_id = NEW.cabinet_id AND d.id = NEW.dossier_id
                      AND d.devise_tenue = NEW.devise) THEN
        RAISE EXCEPTION 'compte de trésorerie : la devise doit être celle de tenue du dossier'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION compte_tresorerie_devise_de_tenue() FROM PUBLIC;

CREATE TRIGGER compte_tresorerie_devise_de_tenue
    BEFORE INSERT ON compte_tresorerie
    FOR EACH ROW EXECUTE FUNCTION compte_tresorerie_devise_de_tenue();

-- Réciproque : la devise de tenue d'un dossier qui a un compte de trésorerie ne change plus (le
-- compte la porte). Déclencheur **distinct** de `dossier_contexte_fige` (0206, 0208, 0209) : une
-- voie de L3 n'en remplace pas la fonction, pour ne pas effacer la règle d'une autre.
CREATE FUNCTION dossier_devise_figee_tresorerie() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.devise_tenue, NEW.decimales_tenue)
           IS DISTINCT FROM (OLD.devise_tenue, OLD.decimales_tenue)
       AND EXISTS (SELECT 1 FROM public.compte_tresorerie t
                    WHERE t.cabinet_id = OLD.cabinet_id AND t.dossier_id = OLD.id) THEN
        RAISE EXCEPTION 'devise de tenue figée : le dossier a des comptes de trésorerie'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION dossier_devise_figee_tresorerie() FROM PUBLIC;

CREATE TRIGGER dossier_devise_figee_tresorerie
    BEFORE UPDATE ON dossier
    FOR EACH ROW EXECUTE FUNCTION dossier_devise_figee_tresorerie();
