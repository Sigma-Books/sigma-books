-- Pièces justificatives (ADR 0015 ; L2 PR 12, issue L2-24). Plage de la voie « pièces » :
-- 0300 à 0399 (crates/api/README.md, « Migrations »).
--
-- Une ligne par pièce **déposée dans un dossier** : jamais partagée entre dossiers ni entre
-- cabinets (spec §2 principe 12, §6.5). Le contenu est dans le stockage objet WORM ; la ligne
-- en garde l'empreinte SHA-256, la clé et la version de l'objet, et la rétention posée.
-- L'objet est écrit **avant** la ligne, dans la transaction de la requête : un objet sans ligne
-- n'est jamais servi.
--
-- Droits du rôle applicatif : SELECT, INSERT, et UPDATE des **seules** colonnes de l'écart
-- (dépôt erroné, ADR 0015). Aucun DELETE. Un écart est définitif (déclencheur ci-dessous).
--
-- Le lien N-N entre pièces et écritures (`lien_piece_ecriture`, rôle principale ou
-- complémentaire, spec §6.5) dépend des tables d'écritures du brouillard et du journal, qui
-- n'existent pas encore : il arrive avec la PR 13 (plage 0200 à 0299), qui ajoutera aussi le
-- refus d'écarter une pièce liée à une écriture validée.

CREATE TABLE piece (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL UNIQUE,
    nature text NOT NULL CHECK (nature IN ('externe', 'interne')),
    -- Type reconnu par la signature (externe) ; forme canonique de la note d'OD (interne).
    type_contenu text NOT NULL CHECK (type_contenu IN (
        'application/pdf', 'image/jpeg', 'image/png', 'text/plain; charset=utf-8'
    )),
    taille bigint NOT NULL CHECK (taille > 0),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    -- Métadonnée seulement, jamais un chemin de stockage.
    nom_origine text CHECK (
        octet_length(nom_origine) BETWEEN 1 AND 255 AND nom_origine !~ '[[:cntrl:]]'
    ),
    -- `<cabinet>/<dossier>/<UUID v4>` : ni l'empreinte, ni le nom de fichier.
    cle_objet text NOT NULL UNIQUE CHECK (
        cle_objet ~ ('^' || cabinet_id::text || '/' || dossier_id::text
                     || '/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$')
    ),
    version_objet text NOT NULL CHECK (octet_length(version_objet) BETWEEN 1 AND 1024),
    retenue_jusqu_au timestamptz NOT NULL,
    deposee_par uuid NOT NULL REFERENCES utilisateur (id),
    deposee_le timestamptz NOT NULL,
    ecartee_le timestamptz,
    ecartee_par uuid REFERENCES utilisateur (id),
    motif_ecart text CHECK (
        octet_length(motif_ecart) BETWEEN 1 AND 1000 AND motif_ecart !~ '[[:cntrl:]]'
        AND length(btrim(motif_ecart)) > 0
    ),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    CHECK ((nature = 'interne') = (type_contenu = 'text/plain; charset=utf-8')),
    -- Rétention d'au moins dix ans (AUDCIF art. 24 ; CGI art. 637).
    CHECK (retenue_jusqu_au >= deposee_le + interval '10 years'),
    CHECK ((ecartee_le IS NULL) = (ecartee_par IS NULL)
           AND (ecartee_le IS NULL) = (motif_ecart IS NULL)),
    CHECK (ecartee_le IS NULL OR ecartee_le >= deposee_le)
);
-- Liste paginée (plus récentes d'abord) et recherche des doublons, dans le dossier seulement.
CREATE INDEX piece_liste ON piece (cabinet_id, dossier_id, deposee_le DESC, id DESC);
CREATE INDEX piece_empreinte ON piece (cabinet_id, dossier_id, empreinte);

ALTER TABLE piece ENABLE ROW LEVEL SECURITY;
ALTER TABLE piece FORCE ROW LEVEL SECURITY;
CREATE POLICY piece_tenant ON piece
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT ON piece TO sigma_application;
GRANT UPDATE (ecartee_le, ecartee_par, motif_ecart) ON piece TO sigma_application;

-- Un écart est définitif : une pièce écartée ne se « rétablit » pas, et aucune autre colonne ne
-- change, quel que soit le rôle (le rôle applicatif n'a déjà l'UPDATE que des colonnes d'écart).
CREATE FUNCTION piece_ecart_definitif() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
BEGIN
    IF OLD.ecartee_le IS NOT NULL THEN
        RAISE EXCEPTION 'pièce déjà écartée' USING ERRCODE = 'check_violation';
    END IF;
    IF (NEW.cabinet_id, NEW.dossier_id, NEW.id, NEW.nature, NEW.type_contenu, NEW.taille,
        NEW.empreinte, NEW.nom_origine, NEW.cle_objet, NEW.version_objet, NEW.retenue_jusqu_au,
        NEW.deposee_par, NEW.deposee_le)
       IS DISTINCT FROM
       (OLD.cabinet_id, OLD.dossier_id, OLD.id, OLD.nature, OLD.type_contenu, OLD.taille,
        OLD.empreinte, OLD.nom_origine, OLD.cle_objet, OLD.version_objet, OLD.retenue_jusqu_au,
        OLD.deposee_par, OLD.deposee_le) THEN
        RAISE EXCEPTION 'pièce immuable' USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION piece_ecart_definitif() FROM PUBLIC;

CREATE TRIGGER piece_ecart_definitif
    BEFORE UPDATE ON piece
    FOR EACH ROW EXECUTE FUNCTION piece_ecart_definitif();
