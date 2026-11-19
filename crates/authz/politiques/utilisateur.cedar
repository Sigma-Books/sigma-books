// Autorisations (`permit`) des actions de l'utilisateur sur lui-même
// (ADR 0012 « Sessions » : « l'utilisateur […] voit les sessions actives et
// les révoque » ; spec §21).
//
// Le principal est l'utilisateur authentifié (`Sigma::Utilisateur`), jamais
// une appartenance : une session n'appartient à aucun cabinet. Le titulaire
// de la session est lu en base, sous RLS, par l'API. Les invariants
// `invariant-titulaire-session` et `invariant-soi-meme` (invariants.cedar)
// interdisent en outre toute action sur ce qui est à un autre.

// Sa session : la consulter, en tourner le jeton, la révoquer (déconnexion
// comprise). Seulement si elle est la sienne.
@id("adr0012-sa-session")
permit (
  principal is Sigma::Utilisateur,
  action in [
    Sigma::Action::"ConsulterSaSession",
    Sigma::Action::"TournerSaSession",
    Sigma::Action::"RevoquerSaSession"
  ],
  resource is Sigma::Session
)
when { resource.titulaire == principal };

// La liste de ses sessions : la ressource est l'utilisateur lui-même.
@id("adr0012-ses-sessions")
permit (
  principal is Sigma::Utilisateur,
  action == Sigma::Action::"ListerSesSessions",
  resource is Sigma::Utilisateur
)
when { resource == principal };
