-- Versions de paramètres fiscaux propres à un dossier (ADR 0030) : identifiant stable de la
-- ligne (cible de la trace d'audit), motif de la surcharge (texte libre, jamais cité par l'audit)
-- et borne du nombre de versions par dossier (50), seconde barrière de la règle du cas d'usage.
-- La table est vide à ce stade (aucune route ne l'alimentait) : les colonnes sont `NOT NULL`
-- sans valeur conservée.
ALTER TABLE parametres_fiscaux_dossier
    ADD COLUMN id uuid NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN motif text NOT NULL DEFAULT 'sans motif'
        CHECK (
            octet_length(motif) BETWEEN 1 AND 400
            AND length(btrim(motif)) >= 1
            AND motif !~ '[[:cntrl:]]'
        ),
    ADD CONSTRAINT parametres_fiscaux_dossier_id_cle UNIQUE (id),
    ADD CONSTRAINT parametres_fiscaux_dossier_numero_max CHECK (numero <= 50);
ALTER TABLE parametres_fiscaux_dossier
    ALTER COLUMN id DROP DEFAULT,
    ALTER COLUMN motif DROP DEFAULT;
