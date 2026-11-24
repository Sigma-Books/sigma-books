// Autorisations (`permit`) et interdictions (`forbid`) du domaine « facteurs » : connexion,
// méthode de secours, codes de récupération, ré-authentification, événements
// d'authentification (ADR 0012, ADR 0017 ; PR 8b, 8c).
//
// La ressource d'une action de l'utilisateur sur ses facteurs est toujours l'utilisateur
// lui-même : l'invariant `invariant-soi-meme` (invariants.cedar) interdit toute action sur un
// autre. La nature et la méthode de la session viennent de la base (session lue sous RLS) ou,
// pour `OuvrirSession`, du facteur que l'API vient de vérifier.

// Connexion (ADR 0012) : action publique et bornée. Le demandeur n'a pas de session ; il ne
// reçoit rien avant d'avoir prouvé un facteur, et la limitation de débit borne les essais.
// La décision qui compte suit la vérification du facteur (`OuvrirSession`).
@id("adr0012-connexion-publique")
permit (
  principal is Sigma::Anonyme,
  action == Sigma::Action::"SeConnecter",
  resource is Sigma::Instance
);

// Ouverture d'une session : une passkey (vérification de l'utilisateur exigée, ADR 0012) ou la
// méthode de secours (mot de passe et TOTP) ouvre une session complète ; un code de
// récupération, une session restreinte (enrôlement seulement, F1.12).
@id("adr0012-ouverture-session")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"OuvrirSession",
  resource is Sigma::Utilisateur
)
when {
  resource == principal &&
  (((context.methode == "passkey" || context.methode == "secours") &&
    context.nature == "complete") ||
   (context.methode == "code_recuperation" && context.nature == "restreinte"))
};

// Un code de récupération n'ouvre jamais qu'une session restreinte (ADR 0012, F1.12), quelles
// que soient les autorisations ajoutées plus tard.
@id("invariant-recuperation-restreinte")
forbid (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"OuvrirSession",
  resource is Sigma::Utilisateur
)
when { context.methode == "code_recuperation" && context.nature != "restreinte" };

// Ré-authentification (secours ou passkey) : depuis une session complète seulement.
@id("adr0012-reauthentification")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"SeReauthentifier",
  resource is Sigma::Utilisateur
)
when { resource == principal && context.nature == "complete" };

// Ses facteurs et ses événements d'authentification (ADR 0017) : session complète.
@id("adr0017-ses-facteurs-et-evenements")
permit (
  principal is Sigma::Utilisateur,
  action in [
    Sigma::Action::"ConsulterSesFacteurs",
    Sigma::Action::"ConsulterSesEvenements"
  ],
  resource is Sigma::Utilisateur
)
when { resource == principal && context.nature == "complete" };

// Enrôlement de la méthode de secours : depuis une session complète « fraîche » (gestion de
// ses propres facteurs, ADR 0012), ou depuis la session restreinte ouverte par un code de
// récupération, qui ne sert qu'à cela (F1.12 ; en L2, jusqu'aux passkeys de la PR 8c, le
// nouveau facteur est la méthode de secours).
@id("adr0012-enrolement-secours")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"EnrolerSecours",
  resource is Sigma::Utilisateur
)
when {
  resource == principal &&
  ((context.nature == "complete" && context.session_fraiche) ||
   (context.nature == "restreinte" && context.methode == "code_recuperation"))
};

// Nouvelle série de codes de récupération : session complète et fraîche.
@id("adr0012-codes-recuperation")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"RegenererCodesRecuperation",
  resource is Sigma::Utilisateur
)
when { resource == principal && context.nature == "complete" && context.session_fraiche };

// Enrôlement d'une passkey : mêmes conditions que celui du secours (session complète fraîche,
// ou session restreinte ouverte par un code de récupération, qui ne sert qu'à enrôler un
// facteur, F1.12). Une passkey ajoutée depuis une session non fraîche affaiblirait la
// protection du compte : le facteur de l'attaquant serait déjà là.
@id("adr0012-enrolement-passkey")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"EnrolerPasskey",
  resource is Sigma::Utilisateur
)
when {
  resource == principal &&
  ((context.nature == "complete" && context.session_fraiche) ||
   (context.nature == "restreinte" && context.methode == "code_recuperation"))
};

// Suppression d'une de ses passkeys : session complète et fraîche.
@id("adr0012-suppression-passkey")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"SupprimerPasskey",
  resource is Sigma::Utilisateur
)
when { resource == principal && context.nature == "complete" && context.session_fraiche };
