# 0029 — Règles de gestion alignées sur Odoo et Tryton : trois assouplissements, deux renvois (format canonique figé)

- **Statut :** proposée
- **Date :** 2026-11-26

## Contexte
Principe du mainteneur : si Odoo et Tryton, validés par leur communauté, ont à peu près le même mécanisme, Sigma fait pareil ; les textes OHADA priment seulement s'ils tranchent explicitement ; Sigma n'ajoute pas de contrôle **bloquant** plus strict que les moteurs sans raison documentée. Un audit a relevé des règles bloquantes que les moteurs n'ont pas. Règles conservées par décision du mainteneur, non touchées : au plus deux exercices non clôturés, reprise limitée à N et N-1, pièce justificative obligatoire (AUDCIF art. 17), exercice civil par défaut.

## Décision
1. **Échéance de clôture informatique : alerte, plus blocage.** Passée l'échéance (3 mois après la période, 4 pour la dernière : AUDCIF art. 22 3° et art. 23), la validation réussit avec l'avertissement `echeance_de_cloture_depassee` ; l'alerte des éditions existait déjà. Le verrouillage et la clôture informatique par un humain restent bloquants. Le délai est un paramètre (`DelaiCloture`, `DELAI_CLOTURE_PAR_DEFAUT`, porté par `Contexte`), que l'ADR multi-pays (0027) remplacera par le paramètre du pays. Moteurs : Tryton `modules/account/period.py:387-441`, Odoo `addons/account/models/company.py:76-100` et `account_move.py:2844-2860` : aucun délai automatique, seulement des verrous posés par un humain.
2. **Contre-passation datée avant l'origine : acceptée.** Moteurs : Tryton `move.py:399-445`, Odoo `wizard/account_move_reversal.py:17` et `93-100`.
3. **Journal de trésorerie : le compte du journal absent de l'écriture avertit** (`compte_tresorerie_absent`), sans bloquer. Moteurs : Tryton `move.py:449-538`, Odoo `account_move.py:2863-2893` et `account_move_line.py:1500-1516`.
4. **Renvoyés, non faits : ligne à zéro et numéro à largeur variable.** Les moteurs les admettent (Tryton `move.py:489-497` rapproche automatiquement les lignes à zéro, contrainte `credit * debit == 0` l. 1080-1083, tri par longueur puis texte l. 171-173 ; Odoo `sequence_mixin.py:334-346`, numéro qui s'allonge), mais les deux touchent le **format canonique haché**, déclaré figé (`docs/integrite/format-v1.md` : « Ce format est **figé** », l. 5) : `numero=` porte « rang sur 6 chiffres » (l. 22), `montant=` est « strictement positif » (l. 33), `crates/integrite` relit par `NumeroEcriture::parse` (6 chiffres exactement) et `Ligne::en_xof` (montant > 0), et les migrations `0202` et `0203` portent `CHECK (montant > 0)`, `sequence BETWEEN 1 AND 999999` et `numero ~ '…-[0-9]{6}$'`. Les assouplir exige un nouveau format (v5, version de l'écriture canonique et de l'export), des migrations et un amendement de la spec §6.2 (invariant 2) et §6.3 : décision d'architecture à part, hors de cette PR. Le plafond de 999 999 écritures par journal et exercice reste donc en vigueur, ainsi que le refus d'une ligne à zéro.

## Conséquences
- La spec contient encore les anciennes règles : §4.3 (l. 170, « plus aucune validation dans la période »), §6.2 invariants 5 et 7, et §6.3 (opération tardive : « une période dont l'échéance de clôture est passée doit d'abord être clôturée », l. 247). `docs/specification.md` n'est pas modifié ici (il ne l'est que sur instruction explicite) : amendement à demander au mainteneur.
- Les codes d'API `echeance_de_cloture_depassee`, `date_valeur_dans_periode_echue`, `date_anterieure_a_l_origine` et `compte_tresorerie_absent` ne sont plus des refus ; les deux premiers et le dernier subsistent comme avertissements (`avertissements` dans la réponse). Les messages correspondants de `web/src/lib/api/messages.ts` deviennent sans objet ; l'interface n'affiche pas encore les avertissements.

## Alternatives écartées
- Supprimer toute alerte d'échéance : l'alerte coûte peu et garde le fondement de l'art. 22 3°.
- Modifier le format canonique dans la même PR : casserait les vecteurs figés et la vérification des exports existants.
