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
-- `devise` : seulement pour suivre la devise de tenue du dossier (déclencheur
-- `dossier_devise_tresorerie` ci-dessous, contrôlé par `compte_tresorerie_devise_de_tenue`).
GRANT UPDATE (nom, plafond, compte_excedent_id, compte_manquant_id, archive, devise)
    ON compte_tresorerie TO sigma_application;

-- La devise d'un compte de trésorerie est celle de tenue du dossier (ADR 0034 point 1 : un compte
-- en devise étrangère relève de L6). Seconde barrière du contrôle de `api` (422
-- `tresorerie_devise_etrangere`), à l'insertion comme à toute modification de la devise.
-- Fonction ordinaire (droits de l'appelant, sous RLS).
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

-- `AFTER` : la politique RLS (`WITH CHECK`) est appliquée d'abord ; une ligne d'un autre dossier
-- est refusée par elle, non par ce contrôle.
CREATE TRIGGER compte_tresorerie_devise_de_tenue
    AFTER INSERT OR UPDATE OF devise ON compte_tresorerie
    FOR EACH ROW EXECUTE FUNCTION compte_tresorerie_devise_de_tenue();

-- Devise de tenue et comptes de trésorerie (plan : « figés dès le premier mouvement ») :
-- - un compte de trésorerie **sans mouvement** suit la devise de tenue quand elle change (la
--   création d'un compte ne fige rien : aucun compte ne se supprime) ;
-- - dès qu'un compte de trésorerie a un **mouvement** (ligne d'écriture validée sur son compte,
--   ou arrêté de caisse : table `arrete_caisse`, `0541`), la devise de tenue ne change plus.
-- Les écritures et brouillons du dossier figent déjà la devise (`dossier_contexte_fige`, 0206) :
-- la règle propre à la trésorerie est l'arrêté. Déclencheur **distinct** de
-- `dossier_contexte_fige` (une voie de L3 n'en remplace pas la fonction, pour ne pas effacer la
-- règle d'une autre). `AFTER` : la ligne du dossier porte déjà la nouvelle devise quand les
-- comptes la suivent. Fonction ordinaire (droits de l'appelant, sous RLS : le contexte du dossier
-- est exigé par `dossier_contexte_fige` pour tout changement de devise).
CREATE FUNCTION dossier_devise_tresorerie() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF (NEW.devise_tenue, NEW.decimales_tenue)
           IS NOT DISTINCT FROM (OLD.devise_tenue, OLD.decimales_tenue) THEN
        RETURN NEW;
    END IF;
    IF EXISTS (SELECT 1 FROM public.compte_tresorerie t
                WHERE t.cabinet_id = OLD.cabinet_id AND t.dossier_id = OLD.id
                  AND (EXISTS (SELECT 1 FROM public.arrete_caisse a
                                WHERE a.cabinet_id = t.cabinet_id AND a.dossier_id = t.dossier_id
                                  AND a.compte_tresorerie_id = t.id)
                       OR EXISTS (SELECT 1 FROM public.ecriture_validee_ligne l
                                   JOIN public.compte c ON c.cabinet_id = l.cabinet_id
                                                       AND c.dossier_id = l.dossier_id
                                                       AND c.numero = l.compte_numero
                                   WHERE l.cabinet_id = t.cabinet_id
                                     AND l.dossier_id = t.dossier_id
                                     AND c.id = t.compte_id))) THEN
        RAISE EXCEPTION 'devise de tenue figée : un compte de trésorerie a des mouvements'
            USING ERRCODE = 'check_violation';
    END IF;
    UPDATE public.compte_tresorerie SET devise = NEW.devise_tenue
     WHERE cabinet_id = OLD.cabinet_id AND dossier_id = OLD.id
       AND devise IS DISTINCT FROM NEW.devise_tenue;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION dossier_devise_tresorerie() FROM PUBLIC;

CREATE TRIGGER dossier_devise_tresorerie
    AFTER UPDATE ON dossier
    FOR EACH ROW EXECUTE FUNCTION dossier_devise_tresorerie();
