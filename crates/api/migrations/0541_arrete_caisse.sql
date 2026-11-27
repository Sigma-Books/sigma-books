-- Arrêté de caisse (L3-21, PR 11 de `docs/lots/L3.md` ; ADR 0034 point 3 ; AUDCIF fiche 57,
-- p. PDF 428 : « contrôle par procès-verbal de caisse »).
--
-- Le comptage est figé en **pièce interne** (note d'opérations diverses, `piece-interne-v1`,
-- format inchangé) ; l'écart avec le solde comptable est **proposé en brouillon** sur le compte
-- d'écart désigné de la caisse, quand il l'est. Ajout seul : un arrêté ne se modifie ni ne se
-- supprime ; un nouveau comptage est un nouvel arrêté. Aucune écriture validée n'est touchée.
--
-- `brouillon_id` n'a pas de clé étrangère : le brouillon proposé disparaît du brouillard à sa
-- validation (`0203`), ou à sa suppression ; l'arrêté garde la référence de la proposition.
CREATE TABLE arrete_caisse (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    compte_tresorerie_id uuid NOT NULL,
    date_arrete date NOT NULL,
    montant_compte numeric NOT NULL CHECK (
        montant_compte >= 0 AND montant_compte < 100000000000000000000
    ),
    solde_comptable numeric NOT NULL CHECK (abs(solde_comptable) < 10000000000000000000000000),
    ecart numeric NOT NULL CHECK (ecart = montant_compte - solde_comptable),
    piece_id uuid NOT NULL,
    brouillon_id uuid,
    arrete_par uuid NOT NULL REFERENCES utilisateur (id),
    arrete_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_tresorerie_id)
        REFERENCES compte_tresorerie (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, piece_id) REFERENCES piece (cabinet_id, dossier_id, id),
    CHECK (brouillon_id IS NULL OR ecart <> 0)
);
CREATE INDEX arrete_caisse_compte
    ON arrete_caisse (cabinet_id, dossier_id, compte_tresorerie_id, date_arrete);

ALTER TABLE arrete_caisse ENABLE ROW LEVEL SECURITY;
ALTER TABLE arrete_caisse FORCE ROW LEVEL SECURITY;
CREATE POLICY arrete_caisse_tenant ON arrete_caisse
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Ajout seul (catalogue `tresorerie.toml`).
GRANT SELECT, INSERT ON arrete_caisse TO sigma_application;
