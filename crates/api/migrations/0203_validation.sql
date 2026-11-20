-- Validation d'une écriture : écritures validées, chaîne de hachage et compteurs (L2-26, PR 13b ;
-- spec §6.2 à §6.4 ; ADR 0007, 0011, 0015, 0019). Voie comptabilité, plage 0200 à 0299.
--
-- Numéro `0203` dans la plage de la voie : ce fichier ne dépend que de `0200_referentiel.sql`
-- (journal, exercice, compte, tiers). Les liens de pièces, qui dépendent de `piece`
-- (`0300_pieces.sql`, voie pièces), sont dans `0351_ecriture_validee_pieces.sql` (numéro
-- supérieur à celui dont elle dépend, comme `0350_brouillard_pieces.sql` en PR 13a).
--
-- Écritures validées : **ajout seul** pour le rôle applicatif (ni `UPDATE` ni `DELETE`, même sur
-- une colonne) et **aucun état modifiable** (ADR 0011 ; règle absolue 5). L'écriture validée
-- reprend l'identifiant du brouillon qu'elle remplace.
--
-- États dérivés, recalculables depuis la chaîne : `chaine_tete` (dernier rang, dernière
-- empreinte) et `compteur_ecriture` (dernière séquence par journal et exercice). Le rôle
-- applicatif y a `UPDATE` sur ces seules colonnes (PostgreSQL l'exige pour `SELECT … FOR UPDATE`).
-- Ordre des verrous (ADR 0011, `crate::base::validation`) : exercices et périodes, brouillon,
-- pièces, tête de chaîne, compteur, tête d'audit.

-- Tête de chaîne d'un dossier : rang 0 et empreinte de départ (`integrite::genese`) à la
-- création, à la demande (`INSERT … ON CONFLICT DO NOTHING`, puis `SELECT … FOR UPDATE`).
CREATE TABLE chaine_tete (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    rang bigint NOT NULL CHECK (rang >= 0),
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    PRIMARY KEY (cabinet_id, dossier_id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id)
);

ALTER TABLE chaine_tete ENABLE ROW LEVEL SECURITY;
ALTER TABLE chaine_tete FORCE ROW LEVEL SECURITY;
CREATE POLICY chaine_tete_tenant ON chaine_tete
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
GRANT SELECT, INSERT ON chaine_tete TO sigma_application;
GRANT UPDATE (rang, empreinte) ON chaine_tete TO sigma_application;

-- Compteur d'un journal pour un exercice : dernière séquence attribuée (spec §6.3).
CREATE TABLE compteur_ecriture (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    journal_id uuid NOT NULL,
    exercice_id uuid NOT NULL,
    dernier integer NOT NULL CHECK (dernier BETWEEN 0 AND 999999),
    PRIMARY KEY (cabinet_id, dossier_id, journal_id, exercice_id),
    FOREIGN KEY (cabinet_id, dossier_id, journal_id)
        REFERENCES journal (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, exercice_id)
        REFERENCES exercice (cabinet_id, dossier_id, id)
);

ALTER TABLE compteur_ecriture ENABLE ROW LEVEL SECURITY;
ALTER TABLE compteur_ecriture FORCE ROW LEVEL SECURITY;
CREATE POLICY compteur_ecriture_tenant ON compteur_ecriture
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
GRANT SELECT, INSERT ON compteur_ecriture TO sigma_application;
GRANT UPDATE (dernier) ON compteur_ecriture TO sigma_application;

-- Écriture validée. `contenu_canonique` est le texte haché (`integrite`), `format` la version du
-- format canonique qui l'a produit (1, 2 ou 3 : `docs/integrite/format-v1.md` à `format-v4.md`) ;
-- `empreinte = SHA-256(empreinte_precedente ‖ contenu_canonique)`. Le contenu relu en colonnes
-- (lignes comprises) redonne le même texte : le test de rapprochement le vérifie.
CREATE TABLE ecriture_validee (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    journal_id uuid NOT NULL,
    exercice_id uuid NOT NULL,
    sequence integer NOT NULL CHECK (sequence BETWEEN 1 AND 999999),
    numero text NOT NULL CHECK (numero ~ '^[A-Z0-9]{1,8}-[A-Za-z0-9]{1,16}-[0-9]{6}$'),
    date_comptable date NOT NULL,
    date_valeur_comptable date,
    libelle text NOT NULL CHECK (
        length(libelle) <= 200 AND length(btrim(libelle)) >= 1 AND libelle !~ '[[:cntrl:]]'
    ),
    version_parametres text,
    format smallint NOT NULL CHECK (format IN (1, 2, 3)),
    contenu_canonique text NOT NULL,
    empreinte bytea NOT NULL CHECK (octet_length(empreinte) = 32),
    empreinte_precedente bytea NOT NULL CHECK (octet_length(empreinte_precedente) = 32),
    rang bigint NOT NULL CHECK (rang >= 1),
    valide_par uuid NOT NULL REFERENCES utilisateur (id),
    date_validation date NOT NULL,
    valide_le timestamptz NOT NULL,
    -- Contre-passation (PR 13c) : colonne prête, nulle pour une validation ordinaire.
    origine_id uuid,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, journal_id)
        REFERENCES journal (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, exercice_id)
        REFERENCES exercice (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, origine_id)
        REFERENCES ecriture_validee (cabinet_id, dossier_id, id),
    CONSTRAINT ecriture_validee_rang UNIQUE (cabinet_id, dossier_id, rang),
    CONSTRAINT ecriture_validee_sequence
        UNIQUE (cabinet_id, dossier_id, journal_id, exercice_id, sequence),
    CONSTRAINT ecriture_validee_numero
        UNIQUE (cabinet_id, dossier_id, journal_id, exercice_id, numero),
    CHECK (date_valeur_comptable IS NULL OR date_valeur_comptable < date_comptable),
    CHECK (right(numero, 6) = lpad(sequence::text, 6, '0'))
);
-- Liste paginée : curseur strict (date comptable, numéro).
CREATE INDEX ecriture_validee_date
    ON ecriture_validee (cabinet_id, dossier_id, date_comptable, numero);

ALTER TABLE ecriture_validee ENABLE ROW LEVEL SECURITY;
ALTER TABLE ecriture_validee FORCE ROW LEVEL SECURITY;
CREATE POLICY ecriture_validee_tenant ON ecriture_validee
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Ajout seul : ni UPDATE ni DELETE (catalogue `validation.toml`). Aucun verrou de ligne n'est
-- possible sur cette table pour le rôle applicatif ; la tête de chaîne sérialise les validations.
GRANT SELECT, INSERT ON ecriture_validee TO sigma_application;

-- Ligne d'une écriture validée : mêmes colonnes que `brouillon_ligne`, étiquette fiscale
-- comprise (rendue dans le contenu canonique v3).
CREATE TABLE ecriture_validee_ligne (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    ecriture_id uuid NOT NULL,
    position smallint NOT NULL CHECK (position BETWEEN 1 AND 200),
    compte_numero text NOT NULL,
    sens text NOT NULL CHECK (sens IN ('debit', 'credit')),
    montant numeric NOT NULL CHECK (montant > 0 AND montant < 100000000000000000000),
    devise text CHECK (devise ~ '^[A-Z]{3}$' AND devise <> 'XOF'),
    montant_devise numeric CHECK (montant_devise > 0 AND montant_devise < 100000000000000000000),
    cours numeric CHECK (cours > 0 AND cours < 100000000000000000000),
    tiers_id uuid,
    fiscal_nature text CHECK (fiscal_nature IN ('base', 'taxe')),
    fiscal_sens text CHECK (fiscal_sens IN ('collectee', 'deductible')),
    fiscal_regime text CHECK (fiscal_regime ~ '^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$'),
    fiscal_taux numeric(9, 6) CHECK (fiscal_taux > 0 AND fiscal_taux <= 100),
    fiscal_exigibilite text CHECK (fiscal_exigibilite IN ('debit', 'encaissement')),
    fiscal_motif text CHECK (fiscal_motif ~ '^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$'),
    PRIMARY KEY (cabinet_id, dossier_id, ecriture_id, position),
    FOREIGN KEY (cabinet_id, dossier_id, ecriture_id)
        REFERENCES ecriture_validee (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_numero)
        REFERENCES compte (cabinet_id, dossier_id, numero),
    FOREIGN KEY (cabinet_id, dossier_id, tiers_id)
        REFERENCES tiers (cabinet_id, dossier_id, id),
    CHECK ((devise IS NULL) = (montant_devise IS NULL) AND (devise IS NULL) = (cours IS NULL)),
    CHECK ((fiscal_nature IS NULL) = (fiscal_sens IS NULL)
           AND (fiscal_nature IS NULL) = (fiscal_regime IS NULL)),
    CHECK (fiscal_nature IS NOT NULL
           OR (fiscal_taux IS NULL AND fiscal_exigibilite IS NULL AND fiscal_motif IS NULL))
);

ALTER TABLE ecriture_validee_ligne ENABLE ROW LEVEL SECURITY;
ALTER TABLE ecriture_validee_ligne FORCE ROW LEVEL SECURITY;
CREATE POLICY ecriture_validee_ligne_tenant ON ecriture_validee_ligne
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
GRANT SELECT, INSERT ON ecriture_validee_ligne TO sigma_application;
