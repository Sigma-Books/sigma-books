// Autorisations (`permit`) du tableau des rôles de la spec §20, domaine « cabinet » :
// choix du cabinet, gestion, audit du cabinet, création d'un dossier.
//
// Une politique par ligne du tableau (ou par décision Q5.x), découpées par domaine
// (`crates/authz/politiques/roles/`) ; chaque fichier est chargé et validé contre le schéma.
// Elles ne regardent que le rôle : le cloisonnement entre cabinets, l'affectation
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
    Sigma::Action::"ReinitialiserAcces",
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
