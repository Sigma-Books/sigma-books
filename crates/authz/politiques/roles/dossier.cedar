// Autorisations (`permit`) du tableau des rôles de la spec §20, domaine « dossier » :
// accès, paramétrage, pièces, périodes, clôture, éditions, intégrité, audit, exports.
//
// Une politique par ligne du tableau (ou par décision Q5.x), découpées par domaine
// (`crates/authz/politiques/roles/`) ; chaque fichier est chargé et validé contre le schéma.
// Elles ne regardent que le rôle : le cloisonnement entre cabinets, l'affectation
// aux dossiers, la séparation des tâches et la ré-authentification sont
// des interdictions transverses (`invariants.cedar`), qui l'emportent.

// Voir un dossier : tout rôle du cabinet qui a au moins une action sur le
// dossier (§20 « Consulter éditions et états » : les quatre rôles). Le
// dossier doit être affecté, sauf pour l'administrateur (Q5.1).
@id("s20-acceder-dossier")
permit (principal, action == Sigma::Action::"AccederDossier", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};

// §20 « Paramétrer plan, journaux, fiscalité » : administrateur, valideur.
@id("s20-parametrer")
permit (principal, action == Sigma::Action::"ParametrerDossier", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// Modifier le contexte du dossier (pays, devise de tenue, fuseau, langue ; ADR 0027) :
// administrateur du cabinet seul. Le cas d'usage exige en plus une session fraîche quand le
// pays ou la devise changent, et refuse tout changement de devise ou de pays une fois une
// écriture validée.
@id("adr0027-modifier-contexte-dossier")
permit (principal, action == Sigma::Action::"ModifierContexteDossier", resource)
when { principal.role == Sigma::Role::"administrateur" };

// §20 « Déposer des pièces » : administrateur, valideur, collaborateur.
// (Client « ses dossiers » : portail, L4.)
@id("s20-deposer-pieces")
permit (principal, action == Sigma::Action::"DeposerPiece", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur"
  ].contains(principal.role)
};

// §20 « Liquider la TVA, verrouiller une période » (verrouillage) :
// administrateur, valideur.
@id("s20-verrouiller-periode")
permit (principal, action == Sigma::Action::"VerrouillerPeriode", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// §20 « Déverrouiller une période » : administrateur seul.
@id("s20-deverrouiller-periode")
permit (principal, action == Sigma::Action::"DeverrouillerPeriode", resource)
when { principal.role == Sigma::Role::"administrateur" };

// §20 « Clôturer un exercice » : administrateur, valideur. La clôture
// informatique d'une période suit les mêmes rôles (docs/lots/L2.md).
@id("s20-cloturer")
permit (
  principal,
  action in [Sigma::Action::"CloturerPeriode", Sigma::Action::"CloturerExercice"],
  resource
)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// Ouvrir l'exercice suivant : administrateur, valideur, comme la clôture
// d'un exercice (§20) ; décision du mainteneur du 2026-11-19 (#172). La
// ré-authentification est un invariant (invariants.cedar).
@id("decision-2026-11-19-ouvrir-exercice-suivant")
permit (principal, action == Sigma::Action::"OuvrirExerciceSuivant", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// §20 « Consulter éditions et états » : les quatre rôles du cabinet.
@id("s20-consulter-editions")
permit (principal, action == Sigma::Action::"ConsulterEditions", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};

// §20 « Vérifier l'intégrité, consulter l'audit » : administrateur,
// valideur, auditeur, sur les dossiers affectés (Q5.1). Audit de portée
// dossier seulement : la portée cabinet relève de l'administrateur
// (ADR 0016, politique « adr0016-audit-cabinet »).
@id("s20-integrite-audit")
permit (
  principal,
  action in [Sigma::Action::"VerifierIntegrite", Sigma::Action::"ConsulterAudit"],
  resource
)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};

// §20 « Exporter (FEC, profils) » : administrateur, valideur, auditeur.
// L'export complet et l'export d'intégrité exigent en plus la
// ré-authentification (invariants.cedar). L'export de l'audit du cabinet
// est réservé à l'administrateur (ADR 0016).
@id("s20-exporter")
permit (
  principal,
  action in [
    Sigma::Action::"Exporter",
    Sigma::Action::"ExporterComplet",
    Sigma::Action::"ExporterIntegrite"
  ],
  resource
)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};
