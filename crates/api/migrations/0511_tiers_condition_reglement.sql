-- Condition de règlement par défaut d'un tiers (L3-17, PR 8 ; ADR 0031 point 4, ADR 0033 point 3).
--
-- Valeur par défaut de saisie, sans effet comptable (Odoo `property_payment_term_id`,
-- `account_move.py:1080-1087` ; Tryton `customer_payment_term`) : reportée de la PR 7a
-- (`0500_tiers_fiche.sql`), ajoutée ici avec la table qu'elle cite (`0510`). Facultative ;
-- clé étrangère **composite** : la condition appartient au dossier du tiers. Modifiable par la
-- fiche du tiers (droit d'`UPDATE` sur cette seule colonne en plus de ceux de `0500`).
ALTER TABLE tiers
    ADD COLUMN condition_reglement_id uuid,
    ADD CONSTRAINT tiers_condition_reglement_fk
        FOREIGN KEY (cabinet_id, dossier_id, condition_reglement_id)
        REFERENCES condition_reglement (cabinet_id, dossier_id, id);

GRANT UPDATE (condition_reglement_id) ON tiers TO sigma_application;
