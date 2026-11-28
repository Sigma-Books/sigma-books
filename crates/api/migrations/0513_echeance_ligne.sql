-- Échéance initiale d'une ligne d'écriture validée (L3-17, PR 8 ; ADR 0033 point 1).
--
-- Copiée de `brouillon_ligne.date_echeance` (`0512`) par la validation (`base/validation.rs`,
-- unitaire et par lot), dans la transaction qui inscrit l'écriture : **un `INSERT` de plus**, sans
-- effet sur le contenu haché ni sur la chaîne. Une contre-passation ne la recopie pas (repli sur
-- la date comptable). Une ligne sans échéance saisie n'a pas de ligne ici : l'échéance effective
-- est alors la date comptable de l'écriture (Tryton `move.py:776`, ADR 0033 point 2).
--
-- **Ajout seul** (catalogue `echeancier.toml`) : ni `UPDATE` ni `DELETE` pour le rôle applicatif ;
-- une échéance se révise par `echeance_report` (`0514`). Limite assumée (ADR 0033 point 1) : hors
-- de la chaîne, `sigma-verifier` ne la vérifie pas ; sa protection est l'ajout seul et l'audit.
--
-- Clé étrangère **composite** vers la ligne validée (même cabinet, même dossier) : une échéance ne
-- désigne jamais la ligne d'un autre dossier.
CREATE TABLE echeance_ligne (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    ecriture_id uuid NOT NULL,
    position smallint NOT NULL,
    date_echeance date NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, ecriture_id, position),
    FOREIGN KEY (cabinet_id, dossier_id, ecriture_id, position)
        REFERENCES ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position)
);

ALTER TABLE echeance_ligne ENABLE ROW LEVEL SECURITY;
ALTER TABLE echeance_ligne FORCE ROW LEVEL SECURITY;
CREATE POLICY echeance_ligne_tenant ON echeance_ligne
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT ON echeance_ligne TO sigma_application;
