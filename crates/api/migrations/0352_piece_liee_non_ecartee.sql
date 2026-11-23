-- Une pièce liée à une écriture validée ou à un brouillon ne s'écarte pas (L2-26, PR 13b ;
-- ADR 0015 : lien verrouillé à la validation). `api` le refuse déjà (`ecarter_piece`, sous le
-- verrou de la pièce) ; ce déclencheur est la seconde barrière, contre une requête erronée du
-- rôle applicatif ou un `UPDATE` direct du propriétaire.
--
-- Numéro hors de la plage de la voie comptabilité : il dépend de `piece` (`0300`, voie pièces),
-- de `brouillon_piece` (`0350`) et de `ecriture_validee_piece` (`0351`).
--
-- Fonction ordinaire (sans `SECURITY DEFINER`), `EXECUTE` retiré à `PUBLIC`, chemin de
-- recherche fixé : le rôle applicatif la déclenche sous sa propre RLS, qui lui montre les liens
-- de son dossier. Un propriétaire ou un super-utilisateur voit tous les liens.
CREATE FUNCTION piece_liee_non_ecartee() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF OLD.ecartee_le IS NULL AND NEW.ecartee_le IS NOT NULL AND (
        EXISTS (SELECT 1 FROM public.ecriture_validee_piece v
                 WHERE v.cabinet_id = OLD.cabinet_id AND v.dossier_id = OLD.dossier_id
                   AND v.piece_id = OLD.id)
        OR EXISTS (SELECT 1 FROM public.brouillon_piece b
                    WHERE b.cabinet_id = OLD.cabinet_id AND b.dossier_id = OLD.dossier_id
                      AND b.piece_id = OLD.id)
    ) THEN
        RAISE EXCEPTION 'pièce liée : elle ne s''écarte pas' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION piece_liee_non_ecartee() FROM PUBLIC;

CREATE TRIGGER piece_liee_non_ecartee
    BEFORE UPDATE ON piece
    FOR EACH ROW EXECUTE FUNCTION piece_liee_non_ecartee();
