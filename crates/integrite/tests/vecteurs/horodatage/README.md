# Vecteurs d'horodatage

Réponses et jetons **réels**, obtenus le 2026-11-18 auprès de DigiCert (`timestamp.digicert.com`) et de Sectigo (`timestamp.sectigo.com`). Ils ne portent que sur l'empreinte de témoins de dossiers fictifs ; ils ne contiennent aucun secret (seulement des certificats publics et des signatures).

| Fichier | Contenu |
|---|---|
| `temoin.txt` | témoin d'un dossier fictif `demo` (`format-v2.md` §4) |
| `requete-openssl.tsq` | requête produite par `openssl ts -query -data temoin.txt -sha256 -cert` (nonce `0xB17F41C783A660D7`) ; `integrite::requete` la reproduit octet pour octet |
| `digicert.tsr`, `sectigo.tsr` | réponses des deux autorités à cette requête |
| `jeton-sectigo-3.tst` | jeton Sectigo du dossier de référence des tests, après 3 écritures |
| `jeton-digicert-5.tst` | jeton DigiCert du même dossier, après ses 5 écritures |

Contre-vérification avec OpenSSL 3.0 (`openssl ts -verify -partial_chain`) : les jetons DigiCert sont acceptés. Les jetons Sectigo sont refusés par OpenSSL (« ess cert id not found ») : leur attribut `signingCertificate` désigne aussi la version **contresignée** de la racine R46, absente de la chaîne qu'OpenSSL construit avec la racine auto-signée. La RFC 2634 (§5.4) n'impose que le premier identifiant, celui du certificat signataire, que `integrite` contrôle.
