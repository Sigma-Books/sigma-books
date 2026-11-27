// Autorisations (`permit`) du domaine « lettrage » (lot L3, ADR 0032).
//
// Même règle que les autres fichiers de `roles/` : seul le rôle est regardé ; cloisonnement,
// affectation et ré-authentification sont des interdictions transverses (`invariants.cedar`).

// §20 « Lettrer / délettrer » : administrateur, valideur, collaborateur. Le dé-lettrage est une
// ligne de rupture (ADR 0032 point 5) : mêmes rôles que le lettrage. La demande de propositions
// automatiques relève de `Lettrer` (elle ne sert qu'à lettrer). Aucune de ces actions ne modifie
// une écriture validée.
@id("s20-lettrer")
permit (
  principal,
  action in [Sigma::Action::"Lettrer", Sigma::Action::"Delettrer"],
  resource
)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur"
  ].contains(principal.role)
};

// Règles de lettrage automatique et paramètre d'application automatique du dossier : comme
// « Paramétrer plan, journaux, fiscalité » (administrateur, valideur). L'application
// automatique, quand le dossier l'autorise, est faite par l'acteur `systeme` (ADR 0032 point 7),
// hors Cedar ; elle n'écrit que des liens de lettrage, jamais une écriture.
@id("l3-parametrer-lettrage")
permit (principal, action == Sigma::Action::"ParametrerLettrage", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};
