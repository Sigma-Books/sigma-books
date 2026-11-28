-- Révisions d'échéance d'une ligne d'écriture validée (L3-17, PR 8 ; ADR 0033 point 2).
--
-- Report accordé ou erreur de saisie : une **nouvelle ligne** (rang 1, 2…), jamais une
-- modification ; une révision se corrige par une nouvelle. Les moteurs modifient la date en place
-- (Tryton `move.py:1074-1078`, Odoo `account_move_line.py:391`) : écart nécessaire (P3), qui garde
-- l'historique. L'échéance effective à une date D est celle de la dernière révision dont la date
-- d'opération est au plus D (`compta_core::echeance::EcheanceLigne::effective`).
--
-- **Ajout seul** (catalogue `echeancier.toml`). Motif obligatoire, 400 octets au plus, sans
-- caractère de contrôle (le contrôle complet, caractères de format compris, est celui de
-- `compta_core::echeance::MotifRevision`). Le motif n'entre jamais au journal d'audit (texte libre).
-- La révision est refusée quand le résiduel de la ligne est nul (409 `echeance_soldee`).
CREATE TABLE echeance_report (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    ecriture_id uuid NOT NULL,
    position smallint NOT NULL,
    rang integer NOT NULL CHECK (rang >= 1),
    nouvelle_echeance date NOT NULL,
    date_operation date NOT NULL,
    motif text NOT NULL CHECK (
        octet_length(motif) <= 400 AND length(btrim(motif)) >= 1 AND motif !~ '[[:cntrl:]]'
    ),
    auteur uuid NOT NULL REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, ecriture_id, position, rang),
    FOREIGN KEY (cabinet_id, dossier_id, ecriture_id, position)
        REFERENCES ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position)
);

ALTER TABLE echeance_report ENABLE ROW LEVEL SECURITY;
ALTER TABLE echeance_report FORCE ROW LEVEL SECURITY;
CREATE POLICY echeance_report_tenant ON echeance_report
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT ON echeance_report TO sigma_application;
