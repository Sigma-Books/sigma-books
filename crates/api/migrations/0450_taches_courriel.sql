-- Envoi des liens par courriel (L2-19, PR 9b ; ADR 0012, 0022, 0024).
--
-- Voie authentification, mais ce fichier dépend de `tache` (`0400_horodatage.sql`, voie intégrité) :
-- il porte donc un numéro supérieur à celui de la voie dont il dépend, hors de sa plage (règle de
-- `crates/api/README.md`, « Migrations »).
--
-- Nature `courriel` : envoyer à un membre ou à un invité le lien d'une **invitation** ou d'une
-- **réinitialisation** (`objet` : l'identifiant de l'invitation ou de la réinitialisation). Le lien
-- n'est jamais stocké en clair : il est émis par le travailleur au moment de l'envoi (la table
-- `lien_acces` n'en garde que l'empreinte), puis remis dans le courriel ; un envoi qui échoue est
-- rejoué avec un lien neuf, qui ferme le précédent. Aucun courriel, aucun nom, aucun lien dans
-- cette table : des identifiants, des instants, des codes stables.
ALTER TABLE tache ADD COLUMN objet uuid;

ALTER TABLE tache DROP CONSTRAINT tache_nature_check;
ALTER TABLE tache ADD CONSTRAINT tache_nature_check
    CHECK (nature IN ('horodatage', 'notification', 'courriel'));

ALTER TABLE tache DROP CONSTRAINT tache_check;
ALTER TABLE tache ADD CONSTRAINT tache_check CHECK (
    (nature = 'horodatage' AND dossier_cible IS NOT NULL
        AND sujet IN ('quotidien', 'verrouillage', 'cloture'))
    OR (nature = 'notification' AND sujet IN (
        'export_audit_cabinet', 'export_audit_dossier', 'export_integrite',
        'retard_horodatage', 'temoin'))
    OR (nature = 'courriel' AND dossier_cible IS NULL AND objet IS NOT NULL
        AND sujet IN ('invitation', 'reinitialisation'))
);

-- Un seul envoi non terminé par objet (renvoyer une invitation pendant un envoi en cours ne crée
-- pas de doublon), et reprise des envois à échéance.
CREATE UNIQUE INDEX tache_courriel_unique
    ON tache (cabinet_cible, sujet, objet)
    WHERE nature = 'courriel' AND etat <> 'terminee';
CREATE INDEX tache_courriel_echeance
    ON tache (prochaine_tentative)
    WHERE nature = 'courriel' AND etat <> 'terminee';
