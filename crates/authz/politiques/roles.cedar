// Autorisations (`permit`) : le tableau des rôles de la spec §20.
//
// Une politique par ligne du tableau (ou par décision Q5.x). Elles ne
// regardent que le rôle : le cloisonnement entre cabinets, l'affectation
// aux dossiers, la séparation des tâches et la ré-authentification sont
// des interdictions transverses (`invariants.cedar`), qui l'emportent.
//
// Rôles par défaut seulement (Q5.4). Le rôle `client` n'a aucune
// autorisation en L2 : ses actions relèvent du portail (L4).
// Les lignes des lots ultérieurs (reprise de données, lettrage, règles
// apprises, TVA, travaux de fin d'exercice, facturation, demandes) sont
// déclarées dans le schéma sans politique : refusées jusqu'à leur lot.

// Voir le cabinet (choix du cabinet actif, ADR 0012) : tout rôle du cabinet
// (§20 : chacun y a au moins une action). L'appartenance doit être active
// (`invariant-appartenance-active`) ; le client n'a aucune autorisation en L2.
@id("adr0012-acceder-cabinet")
permit (principal, action == Sigma::Action::"AccederCabinet", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};

// §20 « Gérer cabinet, utilisateurs, facturation » : administrateur seul.
// Comprend les changements de droits et la séparation des tâches (Q5.3 :
// « désactivable par l'administrateur »).
@id("s20-gerer-cabinet")
permit (
  principal,
  action in [
    Sigma::Action::"GererCabinet",
    Sigma::Action::"ModifierDroits",
    Sigma::Action::"ActiverSeparationDesTaches",
    Sigma::Action::"DesactiverSeparationDesTaches"
  ],
  resource
)
when { principal.role == Sigma::Role::"administrateur" };

// Audit de portée cabinet (ADR 0014) : consultation, vérification et
// export réservés à l'administrateur (ADR 0016). Cette chaîne inscrit tous
// les dossiers, rôles et affectations du cabinet : l'ouvrir à un valideur
// ou à un auditeur affecté à un seul dossier romprait le cloisonnement
// entre dossiers (spec §2 principe 12 ; Q5.1). L'export exige en plus la
// ré-authentification (invariants.cedar).
@id("adr0016-audit-cabinet")
permit (
  principal,
  action in [
    Sigma::Action::"ConsulterAuditCabinet",
    Sigma::Action::"VerifierAuditCabinet",
    Sigma::Action::"ExporterAuditCabinet"
  ],
  resource
)
when { principal.role == Sigma::Role::"administrateur" };

// §20 « Créer un dossier, reprise de données » : administrateur, valideur.
@id("s20-creer-dossier")
permit (principal, action == Sigma::Action::"CreerDossier", resource)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

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
