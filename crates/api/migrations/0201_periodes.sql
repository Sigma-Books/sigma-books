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
-- applicatif. Politiques **restrictives** (ajoutées aux politiques de tenant par un ET), de forme
-- exacte déclarée au catalogue (`catalogue/referentiel.toml`, liste `irreversible`).
--
-- `WITH CHECK (true)` : sans lui, PostgreSQL reprendrait `USING` pour la nouvelle ligne, et la
-- transition vers le statut clôturé serait elle-même refusée.
--
-- Effet sur les verrous de ligne : `SELECT … FOR UPDATE` ou `FOR SHARE` applique aussi les
-- politiques `UPDATE`. Une ligne clôturée n'est donc **ni verrouillée ni rendue** par une telle
-- requête : un appelant ne déduit rien de l'absence d'une ligne dans un résultat verrouillé, il
-- relit l'état par une lecture simple (ce que font les transitions). Une ligne clôturée n'a pas
-- besoin de verrou : elle ne change plus.
CREATE POLICY exercice_irreversible ON exercice
    AS RESTRICTIVE
    FOR UPDATE
    USING (statut <> 'cloture')
    WITH CHECK (true);

CREATE POLICY periode_irreversible ON periode
    AS RESTRICTIVE
    FOR UPDATE
    USING (statut <> 'cloturee')
    WITH CHECK (true);
