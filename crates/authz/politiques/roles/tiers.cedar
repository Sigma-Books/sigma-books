// Autorisations (`permit`) du domaine « tiers » (lot L3) : fiche, fusion, anonymisation,
// conditions de règlement, échéances, éditions de tiers.
//
// La spec §20 n'a aucune ligne pour ces actions : les rôles sont ceux que le plan propose
// (`docs/lots/L3.md`, point C6 ; ADR 0031, 0033), en rattachant chaque action à la ligne du §20
// la plus proche, jamais au-delà. Comme les autres fichiers de `roles/`, ces politiques ne
// regardent que le rôle : le cloisonnement entre cabinets, l'affectation aux dossiers et la
// ré-authentification sont des interdictions transverses (`invariants.cedar`), qui l'emportent.
// Le rôle `client` n'a aucune action de L3 (portail : L4).

// Liste et fiche des tiers, sans les coordonnées bancaires : comme « Consulter éditions et
// états » (les quatre rôles du cabinet ; plan C6).
@id("l3-c6-consulter-tiers")
permit (principal, action == Sigma::Action::"ConsulterTiers", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};

// Coordonnées bancaires des tiers : mêmes rôles, **auditeur exclu** (minimisation, VP17 ;
// plan C6, ADR 0031 point 8). Le contrôle des comptes n'exige pas le RIB d'un fournisseur.
@id("l3-c6-consulter-coordonnees-bancaires-tiers")
permit (principal, action == Sigma::Action::"ConsulterCoordonneesBancairesTiers", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur"
  ].contains(principal.role)
};

// Créer un tiers, l'archiver ou le désarchiver, et tenir les conditions de règlement du
// dossier : comme « Paramétrer plan, journaux, fiscalité » (administrateur, valideur ;
// plan C6). L'archivage suit la création : c'est la ligne de L2 (`ModifierTiers` du
// référentiel, qui portait `actif`, relevait du paramétrage).
@id("l3-c6-parametrer-tiers")
permit (
  principal,
  action in [
    Sigma::Action::"CreerTiers",
    Sigma::Action::"ArchiverTiers",
    Sigma::Action::"ParametrerConditionsReglement"
  ],
  resource
)
when {
  [Sigma::Role::"administrateur", Sigma::Role::"valideur"].contains(principal.role)
};

// Tenir la fiche courante d'un tiers et réviser l'échéance d'une ligne de tiers : comme
// « Saisir, corriger dans le brouillard » (administrateur, valideur, collaborateur ; plan C6,
// ADR 0033). Ni l'une ni l'autre ne touche une écriture validée : la fiche est hors du contenu
// haché, la révision d'échéance est une ligne en ajout seul. Une coordonnée bancaire modifiée
// est journalisée et alerte l'administrateur (ADR 0031 point 3, F1.26).
@id("l3-c6-tenir-fiche-tiers-et-echeances")
permit (
  principal,
  action in [Sigma::Action::"ModifierTiers", Sigma::Action::"ReviserEcheance"],
  resource
)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur"
  ].contains(principal.role)
};

// Fusion par lien, rupture d'une fusion, anonymisation : administrateur seul, avec
// ré-authentification (invariants.cedar ; plan C6, ADR 0031 points 6 et 7). Comme Tryton, qui
// réserve `Replace` et `Erase` au groupe d'administration des tiers.
@id("l3-c6-fusion-anonymisation-tiers")
permit (
  principal,
  action in [
    Sigma::Action::"FusionnerTiers",
    Sigma::Action::"RompreFusionTiers",
    Sigma::Action::"AnonymiserTiers"
  ],
  resource
)
when { principal.role == Sigma::Role::"administrateur" };

// Éditions de tiers (grand livre et balance auxiliaires, balance âgée, relevé de compte,
// échéancier, lettrages) : comme « Consulter éditions et états » (les quatre rôles ; plan C6).
@id("l3-c6-consulter-editions-tiers")
permit (principal, action == Sigma::Action::"ConsulterEditionsTiers", resource)
when {
  [
    Sigma::Role::"administrateur",
    Sigma::Role::"valideur",
    Sigma::Role::"collaborateur",
    Sigma::Role::"auditeur"
  ].contains(principal.role)
};
