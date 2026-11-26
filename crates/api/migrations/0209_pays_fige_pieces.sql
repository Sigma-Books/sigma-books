-- Le pays d'un dossier ne change plus dès qu'une pièce a été déposée (ADR 0030 point 5) : la
-- rétention posée sur l'objet est celle du paquet du pays du dossier **à la date du dépôt** ; un
-- changement de pays laisserait des pièces sous la durée de l'ancien pays (SN -> CM : dix ans au
-- lieu de douze). Seconde barrière de la règle du cas d'usage ; même fonction que `0206` et
-- `0208`, un cas de plus.
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
    IF NEW.pays IS DISTINCT FROM OLD.pays
       AND EXISTS (SELECT 1 FROM public.piece p
                    WHERE p.cabinet_id = OLD.cabinet_id AND p.dossier_id = OLD.id) THEN
        RAISE EXCEPTION 'pays figé : le dossier a des pièces (rétention du pays)'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
