-- Brouillard persistant et journalisé (L2-25, PR 13a ; spec §6.1, §20 ; note d'étude de la
-- PR 13, §1, §5). Voie comptabilité, plage 0200 à 0299 : ne dépend que de `0200_referentiel.sql`
-- (journal, compte, tiers). Le lien aux pièces (`brouillon_piece`) dépend de `0300_pieces.sql` :
-- il est dans `0350_brouillard_pieces.sql`, appliquée après elle sur une base neuve.
--
-- Un seul état « brouillon » ; `a_revoir` est un drapeau facultatif posé et levé à la main (ou
-- posé par l'IA, lot ultérieur). Il n'empêche jamais la validation unitaire (PR 13b) ; il
-- exclura l'écriture de la validation par lot (PR 13c).
--
-- Un brouillon est **modifiable et supprimable** (spec §6.1) : `DELETE` est donc accordé sur ces
-- tables, et seulement sur elles. Rien ici ne touche une écriture validée. Ni l'équilibre ni la
-- justification ne sont imposés par la base : un brouillon peut être déséquilibré en cours de
-- saisie, et le contrôle se fait à la validation (`EcritureEquilibree`, `compta-core`). La base
-- impose en revanche ce qui est invalide en soi (montant strictement positif, forme des codes).
--
-- Règles communes (vérifiées par le test de catalogue) : `cabinet_id`, `dossier_id`, clés
-- étrangères composites vers `dossier`, `journal`, `compte`, `tiers` ; RLS activée et forcée,
-- politique « cabinet ET dossier du contexte ». Tables dont le nom contient `brouill` :
-- fil-piège de la PR 11b (catalogue `brouillard.toml`, lecture à la clôture d'une période).

CREATE TABLE brouillon (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    journal_id uuid NOT NULL,
    date_comptable date NOT NULL,
    -- Date de valeur comptable (AUDCIF art. 22 4°) : antérieure à la date comptable.
    date_valeur_comptable date,
    libelle text NOT NULL CHECK (
        length(libelle) <= 200 AND length(btrim(libelle)) >= 1 AND libelle !~ '[[:cntrl:]]'
    ),
    a_revoir boolean NOT NULL DEFAULT false,
    -- Qui a posé le drapeau (l'humain, ou l'IA d'un lot ultérieur, acteur distinct) : il ne peut
    -- pas le lever lui-même (contrôle de `api`).
    a_revoir_par uuid REFERENCES utilisateur (id),
    -- Verrou optimiste : une modification porte la version qu'elle a lue.
    version integer NOT NULL DEFAULT 1 CHECK (version >= 1),
    cree_par uuid NOT NULL REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL,
    modifie_par uuid NOT NULL REFERENCES utilisateur (id),
    modifie_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, journal_id)
        REFERENCES journal (cabinet_id, dossier_id, id),
    CHECK (date_valeur_comptable IS NULL OR date_valeur_comptable < date_comptable),
    CHECK (a_revoir = (a_revoir_par IS NOT NULL))
);
-- Liste paginée et lecture à la clôture d'une période (dates comptables du dossier).
CREATE INDEX brouillon_date ON brouillon (cabinet_id, dossier_id, date_comptable, id);

ALTER TABLE brouillon ENABLE ROW LEVEL SECURITY;
ALTER TABLE brouillon FORCE ROW LEVEL SECURITY;
CREATE POLICY brouillon_tenant ON brouillon
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- `UPDATE` colonne par colonne : le contenu, le drapeau, la version et la dernière modification.
-- Ni le créateur ni les instants de création ne changent jamais. `SELECT … FOR UPDATE` (verrou de
-- la modification) exige un droit `UPDATE`.
GRANT SELECT, INSERT, DELETE ON brouillon TO sigma_application;
GRANT UPDATE (journal_id, date_comptable, date_valeur_comptable, libelle, a_revoir,
              a_revoir_par, version, modifie_par, modifie_le)
    ON brouillon TO sigma_application;

-- Ligne de brouillon. Le compte est cité par son numéro (clé composite : la base refuse le
-- compte d'un autre dossier) ; un tiers par son UUID. Montant strictement positif en XOF (la
-- contre-valeur est calculée par `compta-core` pour une ligne en devise). Étiquette fiscale
-- facultative, en colonnes à valeurs stables : tout ou rien pour (nature, sens, régime).
CREATE TABLE brouillon_ligne (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    brouillon_id uuid NOT NULL,
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
    PRIMARY KEY (cabinet_id, dossier_id, brouillon_id, position),
    CONSTRAINT brouillon_ligne_brouillon_fk FOREIGN KEY (cabinet_id, dossier_id, brouillon_id)
        REFERENCES brouillon (cabinet_id, dossier_id, id),
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

ALTER TABLE brouillon_ligne ENABLE ROW LEVEL SECURITY;
ALTER TABLE brouillon_ligne FORCE ROW LEVEL SECURITY;
CREATE POLICY brouillon_ligne_tenant ON brouillon_ligne
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Les lignes se remplacent à la modification (suppression puis insertion) : aucun UPDATE.
GRANT SELECT, INSERT, DELETE ON brouillon_ligne TO sigma_application;

-- Aucune colonne d'instant ni de date : le fil-piège des tables exemptées (catalogue) l'interdit,
-- et l'instant d'une contribution est dans le journal d'audit chaîné.
-- Contributeurs (séparation des tâches, spec §20, Q5.3) : le créateur et chaque utilisateur qui
-- modifie le contenu du brouillon. Cedar les reçoit pour décider la validation (PR 13b). Jamais
-- retirés tant que le brouillon existe, même si leur ligne est retirée ensuite ; supprimés avec
-- lui. Le changement du drapeau « à revoir » n'en fait pas un contributeur (le relecteur doit
-- pouvoir valider).
CREATE TABLE brouillon_contributeur (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    brouillon_id uuid NOT NULL,
    utilisateur_id uuid NOT NULL REFERENCES utilisateur (id),
    PRIMARY KEY (cabinet_id, dossier_id, brouillon_id, utilisateur_id),
    CONSTRAINT brouillon_contributeur_brouillon_fk
        FOREIGN KEY (cabinet_id, dossier_id, brouillon_id)
        REFERENCES brouillon (cabinet_id, dossier_id, id)
);

ALTER TABLE brouillon_contributeur ENABLE ROW LEVEL SECURITY;
ALTER TABLE brouillon_contributeur FORCE ROW LEVEL SECURITY;
CREATE POLICY brouillon_contributeur_tenant ON brouillon_contributeur
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT, DELETE ON brouillon_contributeur TO sigma_application;
