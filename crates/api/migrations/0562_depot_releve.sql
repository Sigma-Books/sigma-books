-- Dépôts de relevés (L3-22, PR 12a de `docs/lots/L3.md` ; ADR 0034 points 4 et 5, décision D3).
--
-- Un dépôt rattache un fichier de relevé, conservé comme **pièce externe** de type `text/csv`
-- (`0560` ; WORM, empreinte SHA-256 du serveur, rétention du paquet du pays), au **compte de
-- trésorerie** visé, avec l'auteur et l'instant du dépôt. Le statut probant est explicite :
-- « importé, non certifié » (D3 : l'authenticité n'est pas prouvée par Sigma Books ; la preuve est
-- l'empreinte et la conservation). Le contenu analysé n'est **pas** importé ici : l'analyse
-- (aperçu) n'écrit rien ; l'import des lignes (`releve_import`, `ligne_releve`) est de la PR 12b.
-- OCA garde de même le fichier en pièce jointe du relevé importé
-- (`account_statement_import_file/wizard/account_statement_import.py:42-46`, `:73-84`).
--
-- L'empreinte et la taille sont celles de la pièce (aucune copie) ; aucun montant ici.
CREATE TABLE depot_releve (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    compte_tresorerie_id uuid NOT NULL,
    piece_id uuid NOT NULL,
    statut_probant text NOT NULL DEFAULT 'importe_non_certifie'
        CHECK (statut_probant = 'importe_non_certifie'),
    depose_par uuid NOT NULL REFERENCES utilisateur (id),
    depose_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_tresorerie_id)
        REFERENCES compte_tresorerie (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, piece_id) REFERENCES piece (cabinet_id, dossier_id, id),
    -- Une pièce est le fichier d'un seul dépôt.
    UNIQUE (cabinet_id, dossier_id, piece_id)
);
-- Liste des dépôts (plus récents d'abord), par compte de trésorerie.
CREATE INDEX depot_releve_liste
    ON depot_releve (cabinet_id, dossier_id, compte_tresorerie_id, depose_le DESC, id DESC);

-- La pièce d'un dépôt est une pièce **externe** de type `text/csv` (seconde barrière du contrôle
-- de `api`). Fonction ordinaire (droits de l'appelant, sous RLS).
CREATE FUNCTION depot_releve_piece_texte() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM public.piece p
                    WHERE p.cabinet_id = NEW.cabinet_id AND p.dossier_id = NEW.dossier_id
                      AND p.id = NEW.piece_id
                      AND p.nature = 'externe' AND p.type_contenu = 'text/csv') THEN
        RAISE EXCEPTION 'dépôt de relevé : la pièce doit être un fichier texte externe'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION depot_releve_piece_texte() FROM PUBLIC;

-- `AFTER` : la politique RLS (`WITH CHECK`) est appliquée d'abord.
CREATE TRIGGER depot_releve_piece_texte
    AFTER INSERT ON depot_releve
    FOR EACH ROW EXECUTE FUNCTION depot_releve_piece_texte();

ALTER TABLE depot_releve ENABLE ROW LEVEL SECURITY;
ALTER TABLE depot_releve FORCE ROW LEVEL SECURITY;
CREATE POLICY depot_releve_tenant ON depot_releve
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Ajout seul (catalogue `releves.toml`) : ni UPDATE ni DELETE.
GRANT SELECT, INSERT ON depot_releve TO sigma_application;
