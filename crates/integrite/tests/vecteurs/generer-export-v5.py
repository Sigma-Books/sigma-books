#!/usr/bin/env python3
# Recalcule, indépendamment du moteur (hashlib seul, aucun secret), le vecteur d'export v5 :
#   python3 generer-export-v5.py > export-v5-reference.txt
# Les écritures sont écrites à la main d'après `docs/integrite/format-v5.md` (écriture v4 : devise
# de tenue CDF, deux décimales), puis chaînées comme `docs/integrite/format-v1.md` §3 :
#   empreinte_0 = SHA-256("sigma-books chaine v1\n" + dossier)
#   empreinte_n = SHA-256(empreinte_{n-1} + écriture canonique n)
# 100,55 EUR x 2827,3333 = 284288,363315, arrondi au centime (demi éloigné de zéro) : 284288,36.
import hashlib, sys
dossier = "dossier-cdf"
tete_ = lambda: None
def piece(o): return o*32
def ecr(numero, date, valid, libelle, pieces, lignes, origine=None, parametres=None):
    t = "ecriture v4\n"
    t += f"numero={numero}\ndate={date}\ndate-validation={valid}\n"
    t += "devise-tenue=CDF\ndecimales-tenue=2\n"
    t += f"libelle={libelle}\n"
    if origine: t += f"origine={origine}\n"
    for p in pieces: t += f"piece={piece(p)}\n"
    for l in lignes:
        t += "ligne\n"
        for k, v in l: t += f"{k}={v}\n"
    return t + "fin\n"
e1 = ecr("VT-2026-000001", "2026-01-15", "2026-01-15", "Vente taxable à 18 %", ["9a"], [
    [("compte","4111"),("sens","D"),("montant","118000.55"),("tiers","C001")],
    [("compte","701"),("sens","C"),("montant","100000.47"),("fiscal-nature","base"),("fiscal-sens","collectee"),("fiscal-regime","IMP18"),("fiscal-taux","18"),("fiscal-exigibilite","debit")],
    [("compte","4431"),("sens","C"),("montant","18000.08"),("fiscal-nature","taxe"),("fiscal-sens","collectee"),("fiscal-regime","IMP18"),("fiscal-taux","18"),("fiscal-exigibilite","debit")],
])
# 100,55 EUR x 2827,3333 = 284288,363315 -> 284288,36 (arrondi au centime, demi éloigné de zéro)
e2 = ecr("OD-2026-000001", "2026-02-01", "2026-02-01", "Achat en euros payé depuis la banque", ["8b"], [
    [("compte","605"),("sens","D"),("montant","284288.36"),("devise","EUR"),("montant-devise","100.55"),("cours","2827.3333")],
    [("compte","521"),("sens","C"),("montant","284288.36"),("devise","EUR"),("montant-devise","100.55"),("cours","2827.3333")],
])
e3 = ecr("VT-2026-000002", "2026-03-05", "2026-03-05", "Contre-passation", ["9a"], [
    [("compte","4111"),("sens","C"),("montant","118000.55"),("tiers","C001")],
    [("compte","701"),("sens","D"),("montant","100000.47"),("fiscal-nature","base"),("fiscal-sens","collectee"),("fiscal-regime","IMP18"),("fiscal-taux","18"),("fiscal-exigibilite","debit")],
    [("compte","4431"),("sens","D"),("montant","18000.08"),("fiscal-nature","taxe"),("fiscal-sens","collectee"),("fiscal-regime","IMP18"),("fiscal-taux","18"),("fiscal-exigibilite","debit")],
], origine="VT-2026-000001")
h = hashlib.sha256(("sigma-books chaine v1\n"+dossier).encode()).digest()
out = f"sigma-books export v5\ndossier={dossier}\n"
for e in (e1,e2,e3):
    h = hashlib.sha256(h + e.encode()).digest()
    out += e + f"empreinte={h.hex()}\n"
out += f"tete={h.hex()}\nnombre=3\nfin-export\n"
sys.stdout.write(out)
