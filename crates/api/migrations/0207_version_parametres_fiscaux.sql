-- Version des paramètres fiscaux (ADR 0030, spec §5.4) : choisie par la date comptable de
-- l'écriture, enregistrée dans l'écriture validée et dans son contenu haché (clé `parametres=`,
-- présente depuis le format v1).
--
-- Forme de l'identifiant : `<pays en minuscules>-<AAAA.N>` pour un paquet de pays (`sn-2025.1`),
-- suivi de `+d<N>` pour une version propre à un dossier (`sn-2025.1+d2`). Nulle quand aucune
-- version ne s'applique à la date (écriture sans étiquette fiscale antérieure à la première
-- version du pays) ; les écritures déjà validées gardent NULL (aucune donnée de production).

-- Brouillon : version appliquée à la dernière saisie (information : la validation la recalcule).
ALTER TABLE brouillon
    ADD COLUMN version_parametres text
        CHECK (version_parametres ~ '^[a-z]{2}-[0-9]{4}\.[0-9]{1,4}(\+d[0-9]{1,9})?$');
GRANT UPDATE (version_parametres) ON brouillon TO sigma_application;

-- Écriture validée : la colonne existe (0203) ; sa forme est contrôlée.
ALTER TABLE ecriture_validee
    ADD CONSTRAINT ecriture_validee_version_parametres_check
        CHECK (version_parametres IS NULL
               OR version_parametres ~ '^[a-z]{2}-[0-9]{4}\.[0-9]{1,4}(\+d[0-9]{1,9})?$');
