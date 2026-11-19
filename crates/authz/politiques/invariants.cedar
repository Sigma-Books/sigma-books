// Invariants transverses : interdictions (`forbid`).
//
// Cedar refuse par défaut (ADR 0005) : seule une politique `permit` autorise
// (voir `roles.cedar`). Une interdiction l'emporte toujours sur une
// autorisation : les règles ci-dessous valent pour tous les rôles,
// administrateur compris, quelles que soient les autorisations ajoutées plus
// tard (rôles dérivés, portail client).
//
// Chaque politique porte un `@id` stable : il est inscrit au journal d'audit
// avec la décision (ADR 0014, clé `politique=`).

// Cloisonnement entre cabinets (spec §2 principes 11 et 12 ; ADR 0011 ;
// F1.7, F1.14). Une appartenance n'agit que dans son propre cabinet : la
// ressource (le cabinet lui-même, un dossier ou une écriture) doit en
// descendre. Un utilisateur membre de deux cabinets n'agit dans l'un qu'au
// titre de son appartenance à celui-là, avec le rôle qu'il y tient.
@id("invariant-cloisonnement-cabinet")
forbid (principal, action, resource)
unless { resource in principal.cabinet };

// Appartenance suspendue (ADR 0014 : suspension d'un utilisateur) : plus
// aucune action dans ce cabinet, quel que soit le rôle.
@id("invariant-appartenance-active")
forbid (principal, action, resource)
unless { principal.actif };

// Affectation aux dossiers (Q5.1 ; F1.7). Hors administrateur, une action
// sur un dossier ou sur une de ses écritures exige une affectation à ce
// dossier, qui appartienne à l'appartenance agissante. Une affectation
// fournie pour un autre dossier ou pour un autre utilisateur ne vaut rien.
// L'administrateur voit tous les dossiers de son cabinet.
@id("invariant-affectation-dossier")
forbid (principal, action, resource)
unless {
  resource is Sigma::Cabinet ||
  principal.role == Sigma::Role::"administrateur" ||
  (context has acces &&
   context.acces.appartenance == principal &&
   resource in context.acces.dossier)
};

// Séparation des tâches (spec §20 ; Q5.3 ; F1.16). Quand le paramètre du
// cabinet est actif (par défaut), personne, administrateur compris, ne
// valide une écriture qu'il a lui-même saisie. Corriger un brouillon,
// c'est en saisir une partie : le créateur et chaque correcteur sont
// contributeurs, sans quoi il suffirait de corriger le brouillon d'autrui
// puis de le valider. La validation par lot passe par chaque écriture
// (`Autorisation::decider_lot`).
@id("invariant-separation-des-taches")
forbid (
  principal,
  action == Sigma::Action::"ValiderEcriture",
  resource is Sigma::EcritureBrouillon
)
when {
  principal.cabinet.separation_des_taches &&
  resource.contributeurs.contains(principal.utilisateur)
};

// Ré-authentification (spec §21 ; ADR 0012) : ces actions exigent une
// session « fraîche » (facteur vérifié depuis moins de 5 minutes).
@id("invariant-reauthentification")
forbid (
  principal,
  action in [
    Sigma::Action::"ValiderParLot",
    Sigma::Action::"CloturerPeriode",
    Sigma::Action::"CloturerExercice",
    Sigma::Action::"DeverrouillerPeriode",
    Sigma::Action::"ExporterComplet",
    Sigma::Action::"ExporterIntegrite",
    Sigma::Action::"ExporterAuditCabinet",
    Sigma::Action::"ModifierDroits",
    Sigma::Action::"DesactiverSeparationDesTaches"
  ],
  resource
)
unless { context.session_fraiche };
