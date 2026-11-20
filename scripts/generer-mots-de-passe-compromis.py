#!/usr/bin/env python3
# Génère la liste embarquée des mots de passe compromis (ADR 0012 ; L2 PR 8b) :
# crates/api/donnees/mots-de-passe-compromis.bin, lue par `api::facteurs::compromis`.
#
# Source : SecLists (MIT, © 2018 Daniel Miessler), au commit épinglé ci-dessous ; chaque
# fichier est vérifié par son SHA-256 avant usage. Attribution : docs/provenance/seclists.md.
#
# Format du fichier produit : suite triée, sans doublon, d'entiers de 64 bits gros-boutistes,
# chacun les 8 premiers octets de SHA-256(PREFIXE ‖ mot de passe normalisé). Aucun mot de passe
# n'y figure en clair. Ne sont gardés que les mots de passe d'au moins 12 caractères (les plus
# courts sont déjà refusés par la longueur minimale). Normalisation, identique à celle de
# `compromis.rs` : blancs ASCII (espace, tabulation, sauts de ligne, saut de page) retirés en
# tête et en fin, suites internes réduites à une espace, lettres ASCII A à Z en minuscules ; une
# ligne qui n'est pas de l'UTF-8 valide est ignorée ; la longueur se compte en caractères
# Unicode, sur la forme normalisée.
#
# Usage (réseau requis ; aucun autre fichier écrit) :
#   scripts/generer-mots-de-passe-compromis.py
# ou, avec des copies locales déjà téléchargées (vérifiées par leur SHA-256) :
#   scripts/generer-mots-de-passe-compromis.py --repertoire <dossier>
# Le fichier produit est reproductible : même source, même sortie, octet pour octet.

import argparse
import hashlib
import os
import re
import sys
import urllib.request

COMMIT = "f90961c9c5be42d80e2f1b6e27b968530918862d"
DEPOT = "https://raw.githubusercontent.com/danielmiessler/SecLists"
SOURCES = [
    (
        "Passwords/Common-Credentials/Pwdb_top-1000000.txt",
        "e9a88f67aafe65496682dc374559ee714e978bee50314767494c3e37a18c9fc8",
    ),
    (
        "Passwords/Common-Credentials/xato-net-10-million-passwords-1000000.txt",
        "424a3e03a17df0a2bc2b3ca749d81b04e79d59cb7aeec8876a5a3f308d0caf51",
    ),
]
PREFIXE = b"sigma-books mots-de-passe-compromis v1\0"
LONGUEUR_MIN = 12
MAJUSCULES = bytes(range(ord("A"), ord("Z") + 1))
MINUSCULES = bytes(range(ord("a"), ord("z") + 1))
TABLE = bytes.maketrans(MAJUSCULES, MINUSCULES)
# Blancs ASCII de `str::split_ascii_whitespace` (Rust) : espace, \t, \n, \x0c, \r.
BLANCS = re.compile(rb"[ \t\n\x0c\r]+")
RACINE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SORTIE = os.path.join(RACINE, "crates", "api", "donnees", "mots-de-passe-compromis.bin")


def lire(chemin_source, repertoire):
    if repertoire:
        with open(os.path.join(repertoire, os.path.basename(chemin_source)), "rb") as f:
            return f.read()
    url = f"{DEPOT}/{COMMIT}/{chemin_source}"
    with urllib.request.urlopen(url, timeout=120) as reponse:
        return reponse.read()


def empreintes(contenu):
    for ligne in contenu.split(b"\n"):
        ligne = ligne.rstrip(b"\r")
        try:
            ligne.decode("utf-8")
        except UnicodeDecodeError:
            continue
        normalise = b" ".join(m for m in BLANCS.split(ligne) if m).translate(TABLE)
        if len(normalise.decode("utf-8")) < LONGUEUR_MIN:
            continue
        yield hashlib.sha256(PREFIXE + normalise).digest()[:8]


def main():
    analyse = argparse.ArgumentParser()
    analyse.add_argument("--repertoire", help="copies locales des fichiers de SecLists")
    options = analyse.parse_args()
    toutes = set()
    for chemin_source, attendu in SOURCES:
        contenu = lire(chemin_source, options.repertoire)
        obtenu = hashlib.sha256(contenu).hexdigest()
        if obtenu != attendu:
            sys.exit(f"{chemin_source} : SHA-256 inattendu ({obtenu})")
        toutes.update(empreintes(contenu))
    os.makedirs(os.path.dirname(SORTIE), exist_ok=True)
    with open(SORTIE, "wb") as f:
        for empreinte in sorted(toutes):
            f.write(empreinte)
    print(f"{len(toutes)} empreintes écrites dans {os.path.relpath(SORTIE, RACINE)}")


if __name__ == "__main__":
    main()
