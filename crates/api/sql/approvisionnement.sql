-- Approvisionnement PostgreSQL de Sigma Books (ADR 0011).
--
-- Crée les trois rôles et la base, puis fixe leurs droits au niveau du cluster.
-- Un rôle se crée au niveau du cluster et le rôle de migration n'a pas CREATEROLE :
-- ce n'est donc pas une migration. Le script est exécuté par un super-utilisateur
-- (en développement et en CI), avant `sqlx migrate run`. Il est idempotent.
--
-- Il ne contient aucun mot de passe : ils sont passés en variables psql.
--
--   psql "<URL du super-utilisateur>" -X -v ON_ERROR_STOP=1 \
--        -v base=sigma_books \
--        -v mdp_migration="$SIGMA_MDP_MIGRATION" \
--        -v mdp_application="$SIGMA_MDP_APPLICATION" \
--        -v mdp_lecture_audit="$SIGMA_MDP_LECTURE_AUDIT" \
--        -f crates/api/sql/approvisionnement.sql
--
-- Rôles :
--   sigma_migration      propriétaire de la base, du schéma et des tables ; `sqlx migrate` seulement.
--   sigma_application    rôle du binaire `api` ; droits posés table par table par les migrations.
--   sigma_lecture_audit  lecture du journal d'audit (ADR 0014) ; droits posés par les migrations.
-- Aucun n'a SUPERUSER, BYPASSRLS, CREATEROLE, CREATEDB ni REPLICATION.

\set ON_ERROR_STOP on

-- Variables obligatoires et non vides.
\if :{?base}
\else
  \echo 'approvisionnement : variable « base » manquante (-v base=...)'
  SELECT 1 / 0 AS variable_manquante;
\endif
\if :{?mdp_migration}
\else
  \echo 'approvisionnement : variable « mdp_migration » manquante'
  SELECT 1 / 0 AS variable_manquante;
\endif
\if :{?mdp_application}
\else
  \echo 'approvisionnement : variable « mdp_application » manquante'
  SELECT 1 / 0 AS variable_manquante;
\endif
\if :{?mdp_lecture_audit}
\else
  \echo 'approvisionnement : variable « mdp_lecture_audit » manquante'
  SELECT 1 / 0 AS variable_manquante;
\endif

SELECT :'mdp_migration' = '' OR :'mdp_application' = '' OR :'mdp_lecture_audit' = '' AS mdp_vide \gset
\if :mdp_vide
  \echo 'approvisionnement : un mot de passe est vide'
  SELECT 1 / 0 AS mot_de_passe_vide;
\endif

-- Rôles : créés s'ils manquent, puis attributs et mot de passe (re)fixés à chaque passage.
SELECT format('CREATE ROLE %I', nom)
FROM (VALUES ('sigma_migration'), ('sigma_application'), ('sigma_lecture_audit')) AS r (nom)
WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = r.nom)
\gexec

ALTER ROLE sigma_migration WITH LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB NOREPLICATION
    PASSWORD :'mdp_migration';
ALTER ROLE sigma_application WITH LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB NOREPLICATION
    PASSWORD :'mdp_application';
ALTER ROLE sigma_lecture_audit WITH LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB NOREPLICATION
    PASSWORD :'mdp_lecture_audit';

-- Base : créée si elle manque, possédée par le rôle de migration (le schéma `public`
-- appartient à `pg_database_owner`, donc à lui).
SELECT format('CREATE DATABASE %I OWNER sigma_migration', :'base')
WHERE NOT EXISTS (SELECT 1 FROM pg_catalog.pg_database WHERE datname = :'base')
\gexec

ALTER DATABASE :"base" OWNER TO sigma_migration;

-- Connexion réservée aux trois rôles ; aucun objet temporaire pour personne d'autre que le propriétaire.
REVOKE ALL ON DATABASE :"base" FROM PUBLIC;
GRANT CONNECT ON DATABASE :"base" TO sigma_migration, sigma_application, sigma_lecture_audit;
