// Autorisations (`permit`) du domaine « trésorerie » (lot L3, ADR 0034) : comptes de
// trésorerie, relevés, rapprochement, prélèvements.
//
// La spec §20 n'a aucune ligne pour ces actions : les rôles sont ceux que le plan propose
// (`docs/lots/L3.md`, point C6), rattachés à la ligne du §20 la plus proche. Seul le rôle est
// regardé ; cloisonnement, affectation et ré-authentification sont des interdictions transverses
// (`invariants.cedar`). Le rôle `client` n'a aucune action de L3.

// Lecture : comptes de trésorerie, relevés importés et leurs lignes, propositions, pointages,
// états de rapprochement figés, contrôle des prélèvements. Comme « Consulter éditions et états »
// (les quatre rôles, auditeur compris : plan C6).
@id("l3-c6-consulter-tresorerie")
permit (principal, action == Sigma::Action::"ConsulterTresorerie", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};

// Comptes de trésorerie, comptes désignés du dossier, profils d'import et règles de suggestion :
// comme « Paramétrer plan, journaux, fiscalité » (administrateur, valideur ; plan C6).
@id("l3-c6-parametrer-tresorerie")
permit (
  principal,
  action in [
    Sigma::Action::"ParametrerTresorerie",
    Sigma::Action::"ParametrerReleves"
  ],
  resource
)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// Dépôt et import de relevés, arrêté de caisse, pointage et sa rupture, proposition de
// prélèvement : comme « Saisir, corriger dans le brouillard » (administrateur, valideur,
// collaborateur ; plan C6). Ces actions n'écrivent que des pièces, des lignes en ajout seul et
// des **brouillons** : aucune ne crée ni ne touche une écriture validée (règle absolue 6).
@id("l3-c6-releves-et-pointage")
permit (
  principal,
  action in [
    Sigma::Action::"DeposerReleve",
    Sigma::Action::"ImporterReleve",
    Sigma::Action::"ArreterCaisse",
    Sigma::Action::"PointerReleve",
    Sigma::Action::"RomprePointage",
    Sigma::Action::"ProposerPrelevement"
  ],
  resource
)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur"
  ].contains(principal.role)
};

// Figer un état de rapprochement : valideur (plan C6, ADR 0034 point 9), et donc
// l'administrateur, que le §20 n'exclut d'aucune ligne du valideur.
@id("l3-c6-figer-rapprochement")
permit (principal, action == Sigma::Action::"FigerRapprochement", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};
