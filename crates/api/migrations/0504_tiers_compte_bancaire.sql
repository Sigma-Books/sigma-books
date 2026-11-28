-- Coordonnées bancaires d'un tiers (lot L3, PR 7a, L3-15 ; ADR 0031 points 3 et 8 ; F1.26, VP17).
--
-- `type` : code d'identifiant bancaire du pack (`[[identifiant_bancaire]]` de
-- `crates/syscohada/data/pays.toml` : RIB, IBAN, portefeuille, autre), contrôlé par l'API.
-- Aucune règle de forme (aucune n'est sourcée). Doublon de numéro chez un autre tiers :
-- **signalé**, jamais refusé (Odoo `res_partner_bank.py:68-92` ; Tryton bloque, `bank.py:224-231` :
-- Sigma retient le plus souple). Odoo compare `acc_number` **brut** ; Sigma compare
-- `numero_normalise` (majuscules ; blancs, espaces insécables et caractères de format Unicode
-- retirés), ce qui ne fait que signaler davantage : un numéro qui ne diffère que par un caractère
-- invisible n'échappe pas au signalement (F1.26, F11.8). `devise` : forme ISO 4217.
--
-- Lecture réservée par Cedar (`ConsulterCoordonneesBancairesTiers` : auditeur exclu). Toute
-- modification est journalisée et alerte les administrateurs du cabinet (sujet de notification
-- `coordonnees_bancaires_tiers`, ci-dessous ; ADR 0025). Régime C : `UPDATE` et `DELETE` accordés.

CREATE TABLE tiers_compte_bancaire (
    cabinet_id uuid NOT NULL,
    dossier_id uuid NOT NULL,
    id uuid NOT NULL,
    tiers_id uuid NOT NULL,
    type text NOT NULL CHECK (type ~ '^[A-Z][A-Z0-9_]{1,31}$'),
    numero text NOT NULL CHECK (length(numero) <= 64 AND length(btrim(numero)) >= 1),
    -- Majuscules ; blancs (espaces insécables compris) et caractères de format Unicode retirés.
    numero_normalise text GENERATED ALWAYS AS (upper(regexp_replace(numero,
        '[[:space:]\u00a0\u1680\u2000-\u200f\u2028-\u202f\u205f-\u2064\u2066-\u206f\u3000'
        || '\ufeff\u00ad\u0600-\u0605\u061c\u06dd\u070f\u0890-\u0891\u08e2\u180e\ufff9-\ufffb\U000e0001\U000e0020-\U000e007f]', '', 'g'))) STORED,
    etablissement text CHECK (length(etablissement) <= 200 AND length(btrim(etablissement)) >= 1),
    devise text CHECK (devise ~ '^[A-Z]{3}$'),
    cree_le timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cabinet_id, dossier_id, id),
    FOREIGN KEY (cabinet_id, dossier_id) REFERENCES dossier (cabinet_id, id),
    FOREIGN KEY (cabinet_id, dossier_id, tiers_id) REFERENCES tiers (cabinet_id, dossier_id, id)
);
CREATE INDEX tiers_compte_bancaire_tiers ON tiers_compte_bancaire (cabinet_id, dossier_id, tiers_id);
CREATE INDEX tiers_compte_bancaire_doublon
    ON tiers_compte_bancaire (cabinet_id, dossier_id, numero_normalise);

ALTER TABLE tiers_compte_bancaire ENABLE ROW LEVEL SECURITY;
ALTER TABLE tiers_compte_bancaire FORCE ROW LEVEL SECURITY;
CREATE POLICY tiers_compte_bancaire_tenant ON tiers_compte_bancaire
    FOR ALL
    USING (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    )
    WITH CHECK (
        cabinet_id = NULLIF(current_setting('sigma.cabinet', true), '')::uuid
        AND dossier_id = NULLIF(current_setting('sigma.dossier', true), '')::uuid
    );

GRANT SELECT, INSERT, DELETE ON tiers_compte_bancaire TO sigma_application;
GRANT UPDATE (type, numero, etablissement, devise) ON tiers_compte_bancaire TO sigma_application;

-- Alerte aux administrateurs : coordonnée bancaire d'un tiers ajoutée, modifiée ou retirée
-- (ADR 0031 point 3, F1.26). Sujet de notification **avec** dossier ; aucune donnée du tiers
-- (ni nom, ni numéro) : le courriel est un texte constant. Reprend la contrainte de
-- `0451_alertes.sql` en y ajoutant ce seul sujet.
ALTER TABLE tache DROP CONSTRAINT tache_check;
ALTER TABLE tache ADD CONSTRAINT tache_check CHECK (
    (nature = 'horodatage' AND dossier_cible IS NOT NULL
        AND sujet IN ('quotidien', 'verrouillage', 'cloture'))
    OR (nature = 'notification' AND sujet IN (
        'export_audit_cabinet', 'export_audit_dossier', 'export_integrite',
        'retard_horodatage', 'temoin',
        'reinitialisation_demandee', 'reinitialisation_annulee', 'reinitialisation_aboutie',
        'droits_modifies', 'separation_desactivee', 'connexion_secours',
        'code_recuperation_utilise', 'coordonnees_bancaires_tiers'))
    OR (nature = 'courriel' AND dossier_cible IS NULL AND objet IS NOT NULL
        AND sujet IN ('invitation', 'reinitialisation'))
);
ALTER TABLE tache ADD CONSTRAINT tache_coordonnees_bancaires_avec_dossier CHECK (
    sujet <> 'coordonnees_bancaires_tiers' OR dossier_cible IS NOT NULL
);
