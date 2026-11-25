-- Alertes par courriel aux administrateurs du cabinet (L2-20, PR 9c ; ADR 0012, 0022, 0025).
--
-- Dépend de `tache` (`0400_horodatage.sql`, voie intégrité) et de sa colonne `objet`
-- (`0450_taches_courriel.sql`) : numéro supérieur, hors de la plage de la voie authentification.
--
-- Nouveaux sujets de notification, tous **sans dossier** (`dossier_cible` nul) ; `objet` désigne
-- l'utilisateur concerné :
-- - `reinitialisation_demandee`, `reinitialisation_annulee`, `reinitialisation_aboutie` : étapes de
--   la réinitialisation d'un accès (ADR 0012, F1.12) ; l'utilisateur concerné est alerté aussi ;
-- - `droits_modifies` : rôle, suspension, rétablissement, affectation, délégation, arrivée d'un
--   membre (question 10a : notification des actions d'administration aux administrateurs) ;
-- - `separation_desactivee` : désactivation de la séparation des tâches (Q5.3) ;
-- - `connexion_secours`, `code_recuperation_utilise` : connexion par la méthode de secours ou par
--   un code de récupération (ADR 0012, ADR 0017).
-- Aucun courriel, aucun nom, aucune donnée du dossier : des identifiants et des codes stables.
ALTER TABLE tache DROP CONSTRAINT tache_check;
ALTER TABLE tache ADD CONSTRAINT tache_check CHECK (
    (nature = 'horodatage' AND dossier_cible IS NOT NULL
        AND sujet IN ('quotidien', 'verrouillage', 'cloture'))
    OR (nature = 'notification' AND sujet IN (
        'export_audit_cabinet', 'export_audit_dossier', 'export_integrite',
        'retard_horodatage', 'temoin',
        'reinitialisation_demandee', 'reinitialisation_annulee', 'reinitialisation_aboutie',
        'droits_modifies', 'separation_desactivee', 'connexion_secours',
        'code_recuperation_utilise'))
    OR (nature = 'courriel' AND dossier_cible IS NULL AND objet IS NOT NULL
        AND sujet IN ('invitation', 'reinitialisation'))
);
ALTER TABLE tache ADD CONSTRAINT tache_alertes_sans_dossier CHECK (
    sujet NOT IN ('reinitialisation_demandee', 'reinitialisation_annulee',
                  'reinitialisation_aboutie', 'droits_modifies', 'separation_desactivee',
                  'connexion_secours', 'code_recuperation_utilise')
    OR dossier_cible IS NULL
);
-- Reprise des notifications à échéance.
CREATE INDEX tache_notification_echeance
    ON tache (prochaine_tentative)
    WHERE nature = 'notification' AND etat <> 'terminee';
