// Autorisations (`permit`) du tableau des rôles de la spec §20, domaine « écritures » :
// brouillard, validation, validation par lot, contre-passation.
//
// Une politique par ligne du tableau (ou par décision Q5.x), découpées par domaine
// (`crates/authz/politiques/roles/`) ; chaque fichier est chargé et validé contre le schéma.
// Elles ne regardent que le rôle : le cloisonnement entre cabinets, l'affectation
// aux dossiers, la séparation des tâches et la ré-authentification sont
// des interdictions transverses (`invariants.cedar`), qui l'emportent.

// §20 « Saisir, corriger dans le brouillard » : administrateur, valideur,
// collaborateur. Ne concerne que le brouillard : aucune action ne modifie
// une écriture validée (spec §2 principe 4 ; F1.15).
@id("s20-saisir-brouillard")
permit (
  principal,
  action in [Sigma::Action::"SaisirBrouillard", Sigma::Action::"CorrigerBrouillard"],
  resource
)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur"
  ].contains(principal.role)
};

// §20 « Valider une écriture » : administrateur, valideur.
@id("s20-valider-ecriture")
permit (principal, action == Sigma::Action::"ValiderEcriture", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// §20 « Valider une écriture » : collaborateur « selon délégation ».
// Q5.2 : la délégation est accordée par dossier ; elle doit figurer sur
// l'affectation de ce collaborateur à ce dossier.
@id("q5-2-valider-par-delegation")
permit (principal, action == Sigma::Action::"ValiderEcriture", resource)
when {
  principal.role == Sigma::Role::"collaborateur" &&
  context has acces &&
  context.acces.appartenance == principal &&
  resource in context.acces.dossier &&
  context.acces.delegation_validation
};

// §20 « Validation par lot » : administrateur, valideur.
@id("s20-validation-par-lot")
permit (principal, action == Sigma::Action::"ValiderParLot", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// §20 « Contre-passer » : administrateur, valideur.
@id("s20-contre-passer")
permit (principal, action == Sigma::Action::"ContrePasser", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};
