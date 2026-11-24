-- Contre-passation (L2-27, PR 13c ; spec §6.1 ; AUDCIF art. 20 ; ADR 0020, 0021). Voie
-- comptabilité, plage 0200 à 0299. Dépend seulement de `0203_validation.sql`.
--
-- Une écriture n'est contre-passée qu'une fois : l'index unique partiel sur `origine_id` est le
-- filet de la sérialisation par la tête de chaîne (aucun verrou de ligne n'est possible sur une
-- écriture validée, ajout seul). Choix de conception : contre-passer une contre-passation reste
-- possible (nouvelle écriture d'origine = la contre-passation), mais jamais deux fois la même.
-- Les contraintes ci-dessous se déclarent par `ALTER TABLE` du propriétaire : la table reste en
-- ajout seul pour le rôle applicatif (ni `UPDATE` ni `DELETE`), aucun droit n'est ajouté.
CREATE UNIQUE INDEX ecriture_validee_origine_unique
    ON ecriture_validee (cabinet_id, dossier_id, origine_id)
    WHERE origine_id IS NOT NULL;

-- Une contre-passation est une opération nouvelle : elle ne reprend pas la date de valeur
-- comptable de l'origine (`compta_core::contre_passer`), et ne se cite pas elle-même.
ALTER TABLE ecriture_validee
    ADD CONSTRAINT ecriture_validee_contre_passation
    CHECK (origine_id IS NULL OR (origine_id <> id AND date_valeur_comptable IS NULL));
