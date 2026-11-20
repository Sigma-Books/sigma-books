-- Référentiel d'un dossier : plan de comptes, journaux, exercices, périodes, tiers minimal
-- (L2-22, PR 11a ; spec §4.2, §4.3, §5.1 à §5.3 ; note d'étude de la PR 11, §1 et §2).
--
-- Voie comptabilité, plage 0200 à 0299 (crates/api/README.md, « Migrations ») : cette migration
-- ne dépend que des tables du socle (0001).
--
-- Règles communes à toutes ces tables (vérifiées par le test de catalogue) :
-- - `cabinet_id`, `dossier_id`, `id` (UUID v4 tiré par `api`), clé primaire
--   (`cabinet_id`, `dossier_id`, `id`) ; clé étrangère composite vers `dossier` ;
-- - RLS activée et forcée, politique `dossier_id = sigma.dossier` : le dossier doit être en
--   contexte (aucune n'est `dossier_facultatif`) ;
-- - aucun DELETE ; UPDATE colonne par colonne, sur l'état modifiable seulement.
--
-- Le plan officiel est **copié** dans chaque dossier à sa création (une ligne par compte),
-- avec la version et l'empreinte du référentiel copié (`dossier_referentiel`) : une correction
-- ultérieure des données de `syscohada` ne change pas silencieusement les dossiers existants.
--
-- Un compte est désigné par son numéro, qui ne change jamais : les journaux, les tiers (et les
-- lignes d'écriture de la PR 13) le citent par une clé étrangère composite
-- (`cabinet_id`, `dossier_id`, `numero`), si bien que la base refuse un compte d'un autre
-- dossier.

-- Référentiel copié dans le dossier, et nature de son premier exercice (une ligne par dossier).
CREATE TABLE dossier_referentiel (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    -- `creation` : entité nouvelle, premier exercice ouvert à son début d'activité (AUDCIF
    -- art. 7) ; `reprise` : entité déjà en activité, exercice civil choisi (art. 7 et 34).
    nature text NOT NULL CHECK (nature IN ('creation', 'reprise')),
    debut_activite date,
    referentiel_version text NOT NULL CHECK (length(btrim(referentiel_version)) BETWEEN 1 AND 200),
    referentiel_empreinte bytea NOT NULL CHECK (octet_length(referentiel_empreinte) = 32),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    CHECK ((nature = 'creation') = (debut_activite IS NOT NULL))
);

ALTER TABLE dossier_referentiel ENABLE ROW LEVEL SECURITY;
ALTER TABLE dossier_referentiel FORCE ROW LEVEL SECURITY;
CREATE POLICY dossier_referentiel_tenant ON dossier_referentiel
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT ON dossier_referentiel TO sigma_application;

-- Compte du plan du dossier. Forme du numéro : celle de `compta_core::NumeroCompte` (tronc de
-- 2 à 4 chiffres, le premier de 1 à 9, puis segments libres `-[A-Z0-9]{1,12}`, 32 caractères
-- au plus). Le parent est le numéro sans son dernier segment, ou sans son dernier chiffre :
-- la base garantit qu'il existe (clé étrangère) et qu'il est bien celui-là (CHECK).
CREATE TABLE compte (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    numero text NOT NULL CHECK (
        length(numero) <= 32 AND numero ~ '^[1-9][0-9]{1,3}(-[A-Z0-9]{1,12})*$'
    ),
    parent_numero text,
    intitule text NOT NULL CHECK (
        length(intitule) <= 200 AND length(btrim(intitule)) >= 1
    ),
    genre text NOT NULL CHECK (genre IN ('officiel', 'sous_compte', 'auxiliaire')),
    sens text NOT NULL CHECK (sens IN ('debiteur', 'crediteur', 'variable')),
    collectif boolean NOT NULL,
    -- Admis comme compte d'un journal de trésorerie (marque du référentiel, ou héritée).
    tresorerie boolean NOT NULL,
    actif boolean NOT NULL,
    -- Types de tiers admis sur ce compte officiel et ses sous-comptes (marque `tiers` du
    -- référentiel copié, fiches de fonctionnement des comptes 40, 41, 42 et 46).
    types_tiers text CHECK (
        types_tiers ~ '^(client|fournisseur|salarie|associe|autre)(,(client|fournisseur|salarie|associe|autre))*$'
    ),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    UNIQUE (cabinet_id, dossier_id, numero),
    CHECK (types_tiers IS NULL OR genre = 'officiel'),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, parent_numero)
        REFERENCES compte (cabinet_id, dossier_id, numero),
    -- Un compte officiel n'a aucun segment libre ; un compte créé par le dossier en a un.
    CHECK ((genre = 'officiel') = (position('-' IN numero) = 0)),
    CHECK (parent_numero IS NOT DISTINCT FROM (
        CASE
            WHEN position('-' IN numero) > 0 THEN regexp_replace(numero, '-[A-Z0-9]+$', '')
            WHEN length(numero) > 2 THEN left(numero, length(numero) - 1)
        END
    ))
);

ALTER TABLE compte ENABLE ROW LEVEL SECURITY;
ALTER TABLE compte FORCE ROW LEVEL SECURITY;
CREATE POLICY compte_tenant ON compte
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Le numéro, le parent, le genre, le sens, les types de tiers et les caractères collectif et de
-- trésorerie sont immuables : seuls l'intitulé et l'état actif changent. L'immuabilité de
-- l'intitulé d'un compte officiel relève de `compta-core` (`CompteOfficielImmuable`) et de
-- `api` (seconde barrière à l'enregistrement) : une politique ne voit pas quelle colonne change.
GRANT SELECT, INSERT ON compte TO sigma_application;
GRANT UPDATE (intitule, actif) ON compte TO sigma_application;

-- Journal du dossier. Le compte de trésorerie est un compte du même dossier (clé composite).
CREATE TABLE journal (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    code text NOT NULL CHECK (code ~ '^[A-Z0-9]{1,8}$'),
    intitule text NOT NULL CHECK (
        length(intitule) <= 200 AND length(btrim(intitule)) >= 1
    ),
    type text NOT NULL CHECK (
        type IN ('achats', 'ventes', 'tresorerie', 'operations_diverses', 'a_nouveaux')
    ),
    admis_en_cloture boolean NOT NULL,
    compte_tresorerie text,
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    UNIQUE (cabinet_id, dossier_id, code),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_tresorerie)
        REFERENCES compte (cabinet_id, dossier_id, numero),
    CHECK ((type = 'tresorerie') = (compte_tresorerie IS NOT NULL)),
    CHECK (NOT admis_en_cloture OR type = 'operations_diverses')
);

ALTER TABLE journal ENABLE ROW LEVEL SECURITY;
ALTER TABLE journal FORCE ROW LEVEL SECURITY;
CREATE POLICY journal_tenant ON journal
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Aucune colonne modifiable en PR 11 : le type, l'admission en clôture et le compte de
-- trésorerie ne changent jamais ; le changement de code dépend des écritures (PR 13).
GRANT SELECT, INSERT ON journal TO sigma_application;

-- Exercice (AUDCIF art. 7) : désigné par son année de clôture, clos un 31 décembre.
CREATE TABLE exercice (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    code text NOT NULL CHECK (code ~ '^[0-9]{4}$'),
    debut date NOT NULL,
    fin date NOT NULL,
    statut text NOT NULL CHECK (statut IN ('ouvert', 'en_cloture', 'cloture')),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    UNIQUE (cabinet_id, dossier_id, code),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    CHECK (debut <= fin),
    CHECK (extract(month FROM fin) = 12 AND extract(day FROM fin) = 31),
    CHECK (code::integer = extract(year FROM fin))
);

ALTER TABLE exercice ENABLE ROW LEVEL SECURITY;
ALTER TABLE exercice FORCE ROW LEVEL SECURITY;
CREATE POLICY exercice_tenant ON exercice
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Aucune colonne modifiable en PR 11a : la PR 11b accordera `UPDATE (statut)` (transitions,
-- et `SELECT … FOR UPDATE` qui l'exige) dans sa propre migration.
GRANT SELECT, INSERT ON exercice TO sigma_application;

-- Période : un mois civil, réduit aux bornes de son exercice (spec §4.3).
CREATE TABLE periode (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    exercice_id uuid NOT NULL,
    debut date NOT NULL,
    fin date NOT NULL,
    statut text NOT NULL CHECK (statut IN ('ouverte', 'verrouillee', 'cloturee')),
    -- Dernier changement de statut (PR 11b) : auteur, instant et, pour un déverrouillage, motif.
    statut_par uuid,
    statut_le timestamptz,
    motif text CHECK (motif IS NULL OR (length(motif) <= 200 AND length(btrim(motif)) >= 1)),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    UNIQUE (cabinet_id, dossier_id, debut),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, exercice_id)
        REFERENCES exercice (cabinet_id, dossier_id, id),
    CHECK (debut <= fin),
    CHECK ((statut_par IS NULL) = (statut_le IS NULL))
);
CREATE INDEX periode_exercice ON periode (cabinet_id, dossier_id, exercice_id);

ALTER TABLE periode ENABLE ROW LEVEL SECURITY;
ALTER TABLE periode FORCE ROW LEVEL SECURITY;
CREATE POLICY periode_tenant ON periode
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Aucune colonne modifiable en PR 11a : la PR 11b accordera
-- `UPDATE (statut, statut_par, statut_le, motif)` dans sa propre migration.
GRANT SELECT, INSERT ON periode TO sigma_application;

-- Tiers minimal (L2, question 6 a) : un rôle et un compte collectif. Un client et un
-- fournisseur sont deux tiers, chacun sur son compte collectif (fiches 40 et 41, AUDCIF
-- art. 34 : aucune compensation). Une ligne d'écriture citera le tiers par son UUID, jamais
-- par son nom, qui peut changer.
CREATE TABLE tiers (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    nom text NOT NULL CHECK (length(nom) <= 200 AND length(btrim(nom)) >= 1),
    type text NOT NULL CHECK (type IN ('client', 'fournisseur', 'salarie', 'associe', 'autre')),
    compte_collectif text NOT NULL,
    actif boolean NOT NULL,
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_collectif)
        REFERENCES compte (cabinet_id, dossier_id, numero)
);
CREATE INDEX tiers_nom ON tiers (cabinet_id, dossier_id, nom, id);

ALTER TABLE tiers ENABLE ROW LEVEL SECURITY;
ALTER TABLE tiers FORCE ROW LEVEL SECURITY;
CREATE POLICY tiers_tenant ON tiers
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Le type et le compte collectif sont figés en L2.
GRANT SELECT, INSERT ON tiers TO sigma_application;
GRANT UPDATE (nom, actif) ON tiers TO sigma_application;
