-- Périodes et exercice suivant (L2-23, PR 11b ; spec §4.2, §4.3, §21 ; note d'étude de la PR 11,
-- §6). Voie comptabilité, plage 0200 à 0299 : ne dépend que de `0200_referentiel.sql`.
--
-- Transitions d'une période (verrouiller, déverrouiller, clôturer) : seules les colonnes de
-- statut changent. Ouverture de l'exercice suivant : insertions seules (déjà accordées).
--
-- `UPDATE` colonne par colonne : il permet aussi `SELECT … FOR UPDATE` et `FOR SHARE`, que
-- prennent les transitions (ordre des verrous : `crate::base::referentiel::periodes`).
GRANT UPDATE (statut) ON exercice TO sigma_application;
GRANT UPDATE (statut, statut_par, statut_le, motif) ON periode TO sigma_application;

-- Irréversibilité, en seconde barrière après `compta-core` : une période clôturée (AUDCIF
-- art. 22 3°) ou un exercice clôturé ne se modifie plus, même par une requête erronée du rôle
-- applicatif. Déclencheurs `BEFORE UPDATE` (déclarés au catalogue, `catalogue/referentiel.toml`,
-- liste `irreversible`) : toute modification d'une ligne clôturée lève l'erreur `SB001`, explicite.
--
-- Pourquoi pas une politique restrictive : `SELECT … FOR UPDATE` et `FOR SHARE` appliquent aussi
-- les politiques `UPDATE`, si bien qu'une ligne clôturée deviendrait **invisible** aux verrous de
-- ligne (une validation qui prend sa période `FOR SHARE` ne la verrait plus). Un déclencheur ne
-- change ni la visibilité ni les verrous.
--
-- Fonctions sans `SECURITY DEFINER`, `EXECUTE` retiré à `PUBLIC` : un déclencheur s'exécute
-- sans que le rôle qui modifie la table ait ce droit.
--
-- Exercice : aucune route ne clôture encore un exercice (L6). Le test d'effet du déclencheur
-- (modification d'un exercice clôturé refusée par `SB001`) viendra avec la route de clôture
-- d'exercice ; sa présence et sa forme sont vérifiées dès maintenant par le test de catalogue.
CREATE FUNCTION exercice_irreversible() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog
AS $$
BEGIN
    IF OLD.statut = 'cloture' THEN
        RAISE EXCEPTION 'exercice clôturé : aucune modification' USING ERRCODE = 'SB001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE EXECUTE ON FUNCTION exercice_irreversible() FROM PUBLIC;

CREATE TRIGGER exercice_irreversible
    BEFORE UPDATE ON exercice
    FOR EACH ROW EXECUTE FUNCTION exercice_irreversible();

CREATE FUNCTION periode_irreversible() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog
AS $$
BEGIN
    IF OLD.statut = 'cloturee' THEN
        RAISE EXCEPTION 'période clôturée : aucune modification' USING ERRCODE = 'SB001';
    END IF;
    RETURN NEW;
END;
$$;
REVOKE EXECUTE ON FUNCTION periode_irreversible() FROM PUBLIC;

CREATE TRIGGER periode_irreversible
    BEFORE UPDATE ON periode
    FOR EACH ROW EXECUTE FUNCTION periode_irreversible();
