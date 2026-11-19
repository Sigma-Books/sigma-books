-- Identifiants et état des entités d'autorisation (crates/authz, ADR 0005 et 0011 ; PR 6).
--
-- Cedar prend pour principal une appartenance (un utilisateur dans un cabinet) et reçoit
-- l'affectation à un dossier comme entité : chacune a besoin d'un identifiant propre, et
-- l'appartenance d'un état « active » (suspension, ADR 0014). L'API les lit en base selon le
-- chemin de la requête, jamais d'après le client (crates/authz/src/lib.rs, « Rappels pour l'API »).
--
-- Identifiants UUID v4 tirés par `api` : aucune valeur par défaut. La valeur par défaut posée
-- à l'ajout de la colonne ne sert qu'aux lignes existantes (bases de développement), puis elle
-- est retirée ; elle remplit la colonne lors de la réécriture de la table, hors de la RLS.

ALTER TABLE appartenance ADD COLUMN id uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE appartenance ALTER COLUMN id DROP DEFAULT;
ALTER TABLE appartenance ADD CONSTRAINT appartenance_id_unique UNIQUE (id);

-- Appartenance suspendue (ADR 0014) : plus aucune action dans ce cabinet (politique Cedar
-- `invariant-appartenance-active`). La suspension elle-même arrive avec l'administration du
-- cabinet (PR 9), qui accordera alors `UPDATE (actif)` au rôle applicatif.
ALTER TABLE appartenance ADD COLUMN actif boolean NOT NULL DEFAULT true;

ALTER TABLE acces_dossier ADD COLUMN id uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE acces_dossier ALTER COLUMN id DROP DEFAULT;
ALTER TABLE acces_dossier ADD CONSTRAINT acces_dossier_id_unique UNIQUE (id);
