-- Conditions de règlement du dossier (L3-17, PR 8 de `docs/lots/L3.md` ; ADR 0033 point 3).
--
-- Voie « tiers et trésorerie », sous-plage 0510 à 0519 (crates/api/README.md, « Migrations »).
--
-- Configuration modifiable et journalisée (régime « C » du plan) : une condition a un code stable
-- (unique dans le dossier, immuable : hors du droit d'`UPDATE`), un libellé et un indicateur
-- d'activité ; ses termes se **remplacent** en bloc à la modification (suppression puis insertion,
-- comme les lignes d'un brouillon). Aucune condition ne se supprime : un tiers peut la citer.
-- Les échéances déjà copiées (`echeance_ligne`, `0513`) sont des **dates** : modifier une
-- condition ne change aucune échéance existante (Tryton et Odoo : idem, le terme est recalculé à
-- la seule saisie).
--
-- Termes (Tryton `account_invoice/payment_term.py:49-80`, Odoo `account_payment_term.py:171-327`) :
-- valeur en pourcentage **du total**, montant fixe ou **solde** (le dernier terme, et lui seul :
-- contrôlé par `compta-core`, `ConditionReglement::nouvelle`) ; délai parmi les quatre types
-- d'Odoo (`:294-299`). Les nombres de jours sont des entiers signés, comme chez les deux moteurs.
-- Le pourcentage et le montant fixe sont des décimaux exacts (`numeric`, règle absolue 2).
CREATE TABLE condition_reglement (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    code text NOT NULL CHECK (code ~ '^[A-Z0-9._-]{1,20}$'),
    libelle text NOT NULL CHECK (
        length(libelle) <= 100 AND length(btrim(libelle)) >= 1 AND libelle !~ '[[:cntrl:]]'
    ),
    actif boolean NOT NULL DEFAULT true,
    cree_le timestamptz NOT NULL,
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    CONSTRAINT condition_reglement_code_unique UNIQUE (cabinet_id, dossier_id, code)
);

ALTER TABLE condition_reglement ENABLE ROW LEVEL SECURITY;
ALTER TABLE condition_reglement FORCE ROW LEVEL SECURITY;
CREATE POLICY condition_reglement_tenant ON condition_reglement
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Aucun `DELETE` : une condition citée par un tiers reste lisible ; elle s'archive (`actif`).
GRANT SELECT, INSERT ON condition_reglement TO sigma_application;
GRANT UPDATE (libelle, actif) ON condition_reglement TO sigma_application;

CREATE TABLE condition_reglement_terme (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    condition_id uuid NOT NULL,
    rang smallint NOT NULL CHECK (rang BETWEEN 1 AND 50),
    valeur text NOT NULL CHECK (valeur IN ('pourcentage', 'fixe', 'solde')),
    -- Pourcentage du total (]0, 100[) ou montant fixe (> 0) ; absent pour le solde.
    montant numeric CHECK (montant > 0 AND montant < 100000000000000000000),
    delai text NOT NULL CHECK (
        delai IN ('jours', 'fin_de_mois', 'fin_du_mois_suivant', 'jour_du_mois_suivant')
    ),
    jours integer NOT NULL,
    -- Jour du mois suivant (0 : fin du mois atteint), pour le seul délai `jour_du_mois_suivant`.
    jour_du_mois smallint CHECK (jour_du_mois BETWEEN 0 AND 31),
    PRIMARY KEY (cabinet_id, dossier_id, condition_id, rang),
    FOREIGN KEY (cabinet_id, dossier_id, condition_id)
        REFERENCES condition_reglement (cabinet_id, dossier_id, id),
    CHECK ((valeur = 'solde') = (montant IS NULL)),
    CHECK (valeur <> 'pourcentage' OR montant < 100),
    CHECK ((delai = 'jour_du_mois_suivant') = (jour_du_mois IS NOT NULL))
);

ALTER TABLE condition_reglement_terme ENABLE ROW LEVEL SECURITY;
ALTER TABLE condition_reglement_terme FORCE ROW LEVEL SECURITY;
CREATE POLICY condition_reglement_terme_tenant ON condition_reglement_terme
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

-- Les termes se remplacent en bloc (configuration) : `DELETE` accordé sur cette seule table de
-- termes, jamais sur la condition ni sur une échéance.
GRANT SELECT, INSERT, DELETE ON condition_reglement_terme TO sigma_application;
