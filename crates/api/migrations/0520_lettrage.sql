-- Lettrage manuel en ajout seul (L3-18, PR 9a de `docs/lots/L3.md` ; ADR 0032 ; spec §10.1).
--
-- Aucune écriture validée n'est touchée (règle absolue 5, P3) : le lettrage s'écrit dans des tables
-- propres, **en ajout seul**. Modèle :
-- - `lettrage` : une **opération** (lettre du compte, rang d'opération, date de l'opération dans le
--   fuseau du dossier, origine, auteur) ;
-- - `lettrage_lien` : un lien **partiel** entre une ligne validée au débit et une ligne validée au
--   crédit du même compte, avec le montant imputé de chaque côté (`NUMERIC`, B1 ; modèle d'Odoo,
--   `account_partial_reconcile.py:15-56`) ;
-- - `lettrage_rupture` : le dé-lettrage est une **ligne de rupture** (date, auteur, motif), jamais
--   une suppression (B2 ; Tryton et Odoo suppriment : écart nécessaire, P3) ;
-- - `compteur_lettrage` : état dérivé (régime « D » du plan) par compte, pris `FOR UPDATE` par toute
--   mutation de lettrage (ADR 0032 point 8, rang 7 de l'ordre des verrous) : dernière lettre tirée
--   et dernier rang d'opération.
-- Le **résiduel** d'une ligne (|montant| moins la somme des liens non rompus qui la citent) est
-- calculé, jamais écrit (ADR 0032 point 1). La relecture rejoue les opérations par **rang**
-- d'opération (`compta_core::lettrage::RegistreLettrage::rejouer`), jamais par date : deux
-- opérations du même jour s'ordonnent sans ambiguïté.
--
-- Isolation (ADR 0011) : `dossier_id` sur chaque table, RLS activée et **forcée**, politique
-- `FOR ALL` exacte (cabinet ET dossier du contexte) en `USING` et `WITH CHECK`, clés étrangères
-- **composites** incluant le dossier (une ligne d'un autre dossier ne peut jamais être liée),
-- `GRANT` minimaux, aucune fonction `SECURITY DEFINER`. Aucun numéro de compte ici : le caractère
-- lettrable vient des données de `syscohada` (marque `lettrable`), lu par l'API.

CREATE TABLE compteur_lettrage (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    compte_id uuid NOT NULL,
    -- Dernière lettre tirée sur le compte (`A`…`Z`, `AA`… : `compta_core::Lettre`) ; aucune avant
    -- le premier lettrage. Une lettre n'est jamais réutilisée, même rompue (ADR 0032 point 4).
    derniere_lettre text CHECK (derniere_lettre ~ '^[A-Z]{1,13}$'),
    -- Dernier rang d'opération (lettrage ou rupture) du compte ; 0 : aucune.
    dernier_rang bigint NOT NULL CHECK (dernier_rang >= 0),
    PRIMARY KEY (cabinet_id, dossier_id, compte_id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_id) REFERENCES compte (cabinet_id, dossier_id, id),
    -- Aucune opération sans lettrage préalable (une rupture suit toujours un lettrage).
    CHECK (derniere_lettre IS NOT NULL OR dernier_rang = 0)
);

ALTER TABLE compteur_lettrage ENABLE ROW LEVEL SECURITY;
ALTER TABLE compteur_lettrage FORCE ROW LEVEL SECURITY;
CREATE POLICY compteur_lettrage_tenant ON compteur_lettrage
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Créé à la demande (comme `chaine_tete`) ; `UPDATE` des seules colonnes de compteur, que
-- PostgreSQL exige pour `SELECT … FOR UPDATE`. Jamais `DELETE`.
GRANT SELECT, INSERT ON compteur_lettrage TO sigma_application;
GRANT UPDATE (derniere_lettre, dernier_rang) ON compteur_lettrage TO sigma_application;

CREATE TABLE lettrage (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    compte_id uuid NOT NULL,
    lettre text NOT NULL CHECK (lettre ~ '^[A-Z]{1,13}$'),
    -- Rang d'opération du compte (persisté : il ordonne la relecture).
    rang bigint NOT NULL CHECK (rang >= 1),
    -- Jour de l'opération dans le fuseau du dossier (ADR 0032 point 3 et amendement du
    -- 2026-11-27) : audit, ordre, refus d'une rupture après clôture (B2) ; les états à une date
    -- lisent la plus haute date des lignes liées, pas celle-ci.
    date_operation date NOT NULL,
    origine text NOT NULL CHECK (origine IN ('manuelle', 'automatique', 'proposition_validee')),
    -- Auteur : l'utilisateur ; aucun pour l'application automatique (acteur `systeme`, PR 9b).
    auteur uuid REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, compte_id) REFERENCES compte (cabinet_id, dossier_id, id),
    -- Cible de la clé de `lettrage_rupture` (rupture et lettrage sur le même compte).
    UNIQUE (cabinet_id, dossier_id, id, compte_id),
    UNIQUE (cabinet_id, dossier_id, compte_id, lettre),
    UNIQUE (cabinet_id, dossier_id, compte_id, rang),
    CHECK ((auteur IS NULL) = (origine = 'automatique'))
);

ALTER TABLE lettrage ENABLE ROW LEVEL SECURITY;
ALTER TABLE lettrage FORCE ROW LEVEL SECURITY;
CREATE POLICY lettrage_tenant ON lettrage
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Ajout seul (catalogue `lettrage.toml`).
GRANT SELECT, INSERT ON lettrage TO sigma_application;

CREATE TABLE lettrage_lien (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    lettrage_id uuid NOT NULL,
    -- Ordre du lien dans l'opération (ordre de l'imputation).
    position smallint NOT NULL CHECK (position BETWEEN 1 AND 1000),
    -- Lignes d'écritures validées : (écriture, position de la ligne).
    debit_ecriture_id uuid NOT NULL,
    debit_position smallint NOT NULL,
    credit_ecriture_id uuid NOT NULL,
    credit_position smallint NOT NULL,
    -- Montant imputé de chaque côté, devise de tenue (débit imputé = crédit imputé par
    -- construction, sans tolérance).
    montant numeric NOT NULL CHECK (montant > 0 AND montant < 100000000000000000000),
    PRIMARY KEY (cabinet_id, dossier_id, lettrage_id, position),
    FOREIGN KEY (cabinet_id, dossier_id, lettrage_id)
        REFERENCES lettrage (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, debit_ecriture_id, debit_position)
        REFERENCES ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position),
    FOREIGN KEY (cabinet_id, dossier_id, credit_ecriture_id, credit_position)
        REFERENCES ecriture_validee_ligne (cabinet_id, dossier_id, ecriture_id, position)
);
-- Résiduels calculés par agrégat (ADR 0032 point 1) : les liens qui citent une ligne.
CREATE INDEX lettrage_lien_debit
    ON lettrage_lien (cabinet_id, dossier_id, debit_ecriture_id, debit_position);
CREATE INDEX lettrage_lien_credit
    ON lettrage_lien (cabinet_id, dossier_id, credit_ecriture_id, credit_position);

ALTER TABLE lettrage_lien ENABLE ROW LEVEL SECURITY;
ALTER TABLE lettrage_lien FORCE ROW LEVEL SECURITY;
CREATE POLICY lettrage_lien_tenant ON lettrage_lien
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Ajout seul (catalogue `lettrage.toml`).
GRANT SELECT, INSERT ON lettrage_lien TO sigma_application;

CREATE TABLE lettrage_rupture (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    lettrage_id uuid NOT NULL,
    compte_id uuid NOT NULL,
    rang bigint NOT NULL CHECK (rang >= 1),
    date_operation date NOT NULL,
    -- Texte de 400 octets au plus, sans caractère de contrôle (ADR 0032 point 5) ; les caractères
    -- de format sont refusés par `compta_core::MotifRupture`. Jamais repris dans l'audit.
    motif text NOT NULL CHECK (
        octet_length(motif) <= 400 AND length(btrim(motif)) >= 1 AND motif !~ '[[:cntrl:]]'
    ),
    auteur uuid NOT NULL REFERENCES utilisateur (id),
    cree_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, lettrage_id, compte_id)
        REFERENCES lettrage (cabinet_id, dossier_id, id, compte_id),
    -- Au plus une rupture par lettrage ; re-lettrer crée un nouveau lettrage.
    UNIQUE (cabinet_id, dossier_id, lettrage_id),
    UNIQUE (cabinet_id, dossier_id, compte_id, rang)
);

ALTER TABLE lettrage_rupture ENABLE ROW LEVEL SECURITY;
ALTER TABLE lettrage_rupture FORCE ROW LEVEL SECURITY;
CREATE POLICY lettrage_rupture_tenant ON lettrage_rupture
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );
-- Ajout seul (catalogue `lettrage.toml`).
GRANT SELECT, INSERT ON lettrage_rupture TO sigma_application;

-- Seconde barrière (ADR 0032 point 8, comme le déclencheur de `0352` pour les pièces), derrière
-- `compta-core` et le verrou du compteur : un lien cite une ligne au débit et une ligne au crédit
-- du compte de son lettrage, et ne rend jamais négatif le résiduel d'une ligne (somme des liens
-- non rompus). Fonction ordinaire (droits de l'appelant, sous RLS : elle ne voit que le dossier du
-- contexte), `EXECUTE` retiré à PUBLIC. Une ligne absente laisse la clé étrangère refuser le lien.
CREATE FUNCTION lettrage_lien_controle() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    numero text;
    cote record;
BEGIN
    SELECT c.numero INTO numero
      FROM public.lettrage t
      JOIN public.compte c ON c.cabinet_id = t.cabinet_id AND c.dossier_id = t.dossier_id
                          AND c.id = t.compte_id
     WHERE t.cabinet_id = NEW.cabinet_id AND t.dossier_id = NEW.dossier_id
       AND t.id = NEW.lettrage_id;
    FOR cote IN
        SELECT l.ecriture_id, l.position, l.sens, l.compte_numero, l.montant, v.sens_attendu
          FROM (VALUES (NEW.debit_ecriture_id, NEW.debit_position, 'debit'),
                       (NEW.credit_ecriture_id, NEW.credit_position, 'credit'))
               AS v (ecriture_id, position, sens_attendu)
          JOIN public.ecriture_validee_ligne l
            ON l.cabinet_id = NEW.cabinet_id AND l.dossier_id = NEW.dossier_id
           AND l.ecriture_id = v.ecriture_id AND l.position = v.position
    LOOP
        IF cote.sens <> cote.sens_attendu OR cote.compte_numero IS DISTINCT FROM numero THEN
            RAISE EXCEPTION 'lien de lettrage : sens ou compte de la ligne incohérent'
                USING ERRCODE = 'check_violation';
        END IF;
        IF cote.montant - NEW.montant - COALESCE((
               SELECT sum(k.montant)
                 FROM public.lettrage_lien k
                WHERE k.cabinet_id = NEW.cabinet_id AND k.dossier_id = NEW.dossier_id
                  AND ((k.debit_ecriture_id = cote.ecriture_id
                        AND k.debit_position = cote.position)
                       OR (k.credit_ecriture_id = cote.ecriture_id
                           AND k.credit_position = cote.position))
                  AND NOT EXISTS (
                      SELECT 1 FROM public.lettrage_rupture r
                       WHERE r.cabinet_id = k.cabinet_id AND r.dossier_id = k.dossier_id
                         AND r.lettrage_id = k.lettrage_id)), 0) < 0 THEN
            RAISE EXCEPTION 'lien de lettrage : résiduel négatif'
                USING ERRCODE = 'check_violation';
        END IF;
    END LOOP;
    RETURN NEW;
END
$$;
REVOKE EXECUTE ON FUNCTION lettrage_lien_controle() FROM PUBLIC;

-- `BEFORE` : le contrôle voit les liens déjà insérés (y compris ceux de la même opération,
-- insérés un à un). La RLS (`WITH CHECK`) et les clés étrangères s'appliquent en plus.
CREATE TRIGGER lettrage_lien_controle
    BEFORE INSERT ON lettrage_lien
    FOR EACH ROW EXECUTE FUNCTION lettrage_lien_controle();
