//! Routes du cabinet et de ses dossiers (PR 6 et 7) : paramètres du cabinet, dossiers visibles,
//! un dossier, et la première mutation, la création d'un dossier, inscrite au journal d'audit
//! dans sa transaction (`crates/api/CLAUDE.md`).

use authz::Action;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Deserializer, Serialize};
use syscohada::PackPays;

use super::{
    ActionRoute, CasUsage, Methode, Module, Portee, Requete, Resultat, Route, referentiel,
};
use crate::base::TransactionTenant;
use crate::base::audit::PorteeAudit;
use crate::base::organisation::{
    a_des_parametres_fiscaux_propres, a_des_pieces, affecter_utilisateur_du_contexte,
    creer_dossier, ecritures_du_dossier, inserer_identifiants_legaux, lire_appartenance,
    lire_cabinet, lire_dossier_visible, lire_identifiants_legaux, lister_dossiers_visibles,
    modifier_contexte, remplacer_identifiants_legaux, verrouiller_dossier_pour_modification,
};
use crate::contexte_dossier::{
    ContexteDossier, IdentifiantLegal, ModificationContexte, controler_identifiants,
    modifier as modifier_le_contexte,
};
use crate::serveur::audit::Trace;
use crate::serveur::erreur::{CategorieErreur, ErreurApi, MotifConflit};
use crate::serveur::pipeline::{CabinetAutorise, DossierAutorise, Garde};

/// Cas d'usage du module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cas {
    /// Paramètres du cabinet.
    LireCabinet,
    /// Dossiers visibles du cabinet.
    ListerDossiers,
    /// Un dossier.
    LireDossier,
    /// Créer un dossier : inscrit au cabinet, puis en première entrée du dossier (ADR 0014) ;
    /// le créateur y est affecté.
    CreerDossier,
    /// Pack pays (ADR 0027) : pays de l'OHADA, valeurs par défaut du dossier (devise de tenue,
    /// fuseau, langue), types d'identifiants légaux, devises et fuseaux admis. Données publiques
    /// du produit, lues pour la page de création d'un dossier.
    ListerPays,
    /// Modifier le contexte du dossier (ADR 0027) : fuseau, langue, forme juridique à tout
    /// moment ; pays et devise de tenue tant que le dossier n'a ni écriture ni brouillon.
    ModifierContexte,
}

/// Tranche du module.
pub const MODULE: Module = Module {
    nom: "organisation",
    routes: ROUTES,
    champs_journal: &[],
};

/// Routes du module.
pub const ROUTES: &[Route] = &[
    // §20 « Gérer cabinet » : paramètres du cabinet, administrateur seul (403 pour les autres).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}",
        action: ActionRoute::Cabinet(Action::GererCabinet),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Organisation(Cas::LireCabinet),
    },
    // Dossiers visibles : tous pour l'administrateur, les dossiers affectés pour les autres (Q5.1).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: ActionRoute::Cabinet(Action::AccederDossier),
        portee: Portee::DossiersDuCabinet,
        cas_usage: CasUsage::Organisation(Cas::ListerDossiers),
    },
    // §20 « Créer un dossier » : administrateur, valideur.
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers",
        action: ActionRoute::Cabinet(Action::CreerDossier),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Organisation(Cas::CreerDossier),
    },
    // Modification du contexte du dossier : administrateur du cabinet (ADR 0027).
    Route {
        methode: Methode::Post,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}/contexte",
        action: ActionRoute::Cabinet(Action::ModifierContexteDossier),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Organisation(Cas::ModifierContexte),
    },
    // Pack pays : mêmes droits que la création d'un dossier (c'est sa page qui s'en sert).
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/pays",
        action: ActionRoute::Cabinet(Action::CreerDossier),
        portee: Portee::Cabinet,
        cas_usage: CasUsage::Organisation(Cas::ListerPays),
    },
    Route {
        methode: Methode::Get,
        chemin: "/api/cabinets/{cabinet}/dossiers/{dossier}",
        action: ActionRoute::Cabinet(Action::AccederDossier),
        portee: Portee::Dossier,
        cas_usage: CasUsage::Organisation(Cas::LireDossier),
    },
];

/// Longueur maximale du nom d'un dossier (contrainte de la table `dossier`).
const NOM_DOSSIER_MAX: usize = 200;

/// Nom de dossier admis : 1 à 200 caractères, non blanc, sans caractère de contrôle. Le corps
/// de la création (nom, ouverture du premier exercice) est lu par le module `referentiel`.
fn nom_dossier(nom: &str) -> Result<String, ErreurApi> {
    let valide = !nom.trim().is_empty()
        && nom.chars().count() <= NOM_DOSSIER_MAX
        && !nom.chars().any(char::is_control);
    if valide {
        Ok(nom.to_owned())
    } else {
        Err(ErreurApi::RequeteInvalide)
    }
}

/// Vue d'un cabinet.
#[derive(Debug, Serialize)]
struct VueCabinet {
    id: String,
    nom: String,
    separation_taches: bool,
}

/// Vue d'un dossier : ne se construit que depuis un [`DossierAutorise`], que seule la [`Garde`]
/// fabrique. Rend le contexte du dossier (ADR 0027) : pays, devise de tenue et son échelle,
/// fuseau, langue, forme juridique. Les identifiants légaux ne sont rendus que par la lecture
/// d'un dossier et sa création, jamais par la liste.
#[derive(Debug, Serialize)]
struct VueDossier {
    id: String,
    nom: String,
    pays: String,
    devise: String,
    decimales: u8,
    fuseau: String,
    langue: String,
    forme_juridique: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identifiants_legaux: Option<Vec<VueIdentifiant>>,
}

/// Un identifiant légal rendu à l'interface.
#[derive(Debug, Serialize)]
struct VueIdentifiant {
    #[serde(rename = "type")]
    type_identifiant: String,
    valeur: String,
}

impl VueDossier {
    fn de(dossier: &DossierAutorise, identifiants: Option<Vec<IdentifiantLegal>>) -> Self {
        Self::depuis(
            dossier.id().to_string(),
            dossier.nom(),
            dossier.contexte(),
            identifiants,
        )
    }

    fn depuis(
        id: String,
        nom: &str,
        c: &ContexteDossier,
        identifiants: Option<Vec<IdentifiantLegal>>,
    ) -> Self {
        Self {
            id,
            nom: nom.to_owned(),
            pays: c.pays.clone(),
            devise: c.devise.clone(),
            decimales: c.decimales,
            fuseau: c.fuseau.clone(),
            langue: c.langue.clone(),
            forme_juridique: c.forme_juridique.clone(),
            identifiants_legaux: identifiants.map(|v| {
                v.into_iter()
                    .map(|i| VueIdentifiant {
                        type_identifiant: i.type_identifiant,
                        valeur: i.valeur,
                    })
                    .collect()
            }),
        }
    }
}

/// Vue du pack pays (`GET /api/cabinets/{cabinet}/pays`).
#[derive(Debug, Serialize)]
struct VuePack {
    pays: Vec<VuePays>,
    devises: Vec<VueDevise>,
    fuseaux: Vec<VueFuseau>,
}

#[derive(Debug, Serialize)]
struct VuePays {
    code: String,
    nom: String,
    devise: String,
    fuseau: String,
    langue: String,
    identifiants: Vec<VueTypeIdentifiant>,
}

#[derive(Debug, Serialize)]
struct VueTypeIdentifiant {
    code: String,
    libelle: String,
}

#[derive(Debug, Serialize)]
struct VueDevise {
    code: String,
    intitule: String,
    decimales: u8,
}

#[derive(Debug, Serialize)]
struct VueFuseau {
    nom: String,
    decalage_minutes: i16,
}

impl VuePack {
    fn de(pack: &PackPays) -> Self {
        Self {
            pays: pack
                .pays()
                .iter()
                .map(|p| VuePays {
                    code: p.code().to_owned(),
                    nom: p.nom().to_owned(),
                    devise: p.devise().to_owned(),
                    fuseau: p.fuseau().to_owned(),
                    langue: p.langue().to_owned(),
                    identifiants: p
                        .identifiants()
                        .iter()
                        .map(|t| VueTypeIdentifiant {
                            code: t.code().to_owned(),
                            libelle: t.libelle().to_owned(),
                        })
                        .collect(),
                })
                .collect(),
            devises: pack
                .devises()
                .iter()
                .map(|d| VueDevise {
                    code: d.code().to_owned(),
                    intitule: d.intitule().to_owned(),
                    decimales: d.decimales(),
                })
                .collect(),
            fuseaux: pack
                .fuseaux()
                .iter()
                .map(|z| VueFuseau {
                    nom: z.nom().to_owned(),
                    decalage_minutes: z.decalage_minutes(),
                })
                .collect(),
        }
    }
}

impl From<CabinetAutorise> for VueCabinet {
    fn from(autorise: CabinetAutorise) -> Self {
        let cabinet = autorise.cabinet();
        Self {
            id: cabinet.id.to_string(),
            nom: cabinet.nom.clone(),
            separation_taches: cabinet.separation_taches,
        }
    }
}

/// Exécute un cas d'usage du module dans la transaction du pipeline.
pub(crate) async fn executer(
    cas: Cas,
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    match cas {
        Cas::LireCabinet => {
            let cabinet = lire_cabinet(tx).await?.ok_or(ErreurApi::Introuvable)?;
            let vue = VueCabinet::from(garde.cabinet_du_chemin(cabinet)?);
            Ok(Resultat::lecture(Json(vue).into_response()))
        }
        Cas::ListerDossiers => {
            // Filtre de la requête (affectation), puis décision Cedar pour chaque dossier.
            let autorises = garde.filtrer(lister_dossiers_visibles(tx).await?)?;
            let vues: Vec<VueDossier> = autorises.iter().map(|d| VueDossier::de(d, None)).collect();
            Ok(Resultat::lecture(Json(vues).into_response()))
        }
        Cas::LireDossier => {
            let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
            let dossier = lire_dossier_visible(tx, id)
                .await?
                .ok_or(ErreurApi::Introuvable)?;
            let autorise = garde.dossier_du_chemin(dossier)?;
            let identifiants = lire_identifiants_legaux(tx).await?;
            let vue = VueDossier::de(&autorise, Some(identifiants));
            Ok(Resultat::lecture(Json(vue).into_response()))
        }
        Cas::CreerDossier => creer(tx, garde, requete).await,
        Cas::ModifierContexte => modifier(tx, garde, requete).await,
        Cas::ListerPays => {
            garde.exiger_cabinet()?;
            exiger_createur(tx).await?;
            let pack =
                PackPays::officiel().map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
            Ok(Resultat::lecture(Json(VuePack::de(pack)).into_response()))
        }
    }
}

/// Défense en profondeur de la création d'un dossier (ADR 0011) : même si Cedar se trompait,
/// la requête exige une appartenance **active**, lue en base, de rôle `administrateur` ou
/// `valideur` (§20 « Créer un dossier »). Appartenance absente ou suspendue : le cabinet est
/// d'autrui (404) ; autre rôle : action refusée sur une ressource visible (403).
async fn exiger_createur(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    let autorises = [authz::Role::Administrateur, authz::Role::Valideur];
    if autorises.iter().any(|r| r.nom() == appartenance.role) {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

/// Création d'un dossier (action du cabinet) : le dossier, l'affectation de son créateur, puis
/// deux traces : au cabinet, puis en première entrée de la chaîne du dossier (ADR 0014).
async fn creer(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    garde.exiger_cabinet()?;
    exiger_createur(tx).await?;
    // Tout le référentiel du dossier (plan, journaux, premier exercice) est calculé en pur
    // avant la première écriture (module `referentiel`, PR 11).
    let demande = referentiel::lire_creation(requete.corps, requete.maintenant)?;
    let nom = nom_dossier(demande.nom())?;
    let dossier = creer_dossier(tx, &nom, demande.contexte()).await?;
    affecter_utilisateur_du_contexte(tx, dossier.id).await?;
    let mut donnees = referentiel::instancier(tx, dossier.id, &demande).await?;
    // La transaction est restreinte au dossier créé (référentiel) : ses identifiants légaux
    // s'inscrivent sous la RLS de dossier.
    inserer_identifiants_legaux(tx, demande.identifiants()).await?;
    // Modèles d'usage des conditions de règlement, éditables (L3-17 ; ADR 0033 point 3).
    let conditions = super::echeancier::creer_modeles_d_usage(tx, requete.maintenant).await?;
    donnees.push(("conditions_reglement", conditions.to_string()));
    let c = demande.contexte();
    donnees.extend([
        ("pays", c.pays.clone()),
        (
            "devise_tenue",
            format!("{} ; {} décimale(s)", c.devise, c.decimales),
        ),
        ("fuseau", c.fuseau.clone()),
        ("langue", c.langue.clone()),
        (
            "identifiants_legaux",
            demande
                .identifiants()
                .iter()
                .map(|i| i.type_identifiant.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
    ]);
    let id = dossier.id.uuid();

    let mut au_cabinet = Trace::new(PorteeAudit::Cabinet, "dossier", id);
    au_cabinet.champs.push(("nom", None, Some(nom.clone())));
    au_cabinet
        .champs
        .push(("affectation", None, Some(garde.utilisateur().to_string())));
    au_cabinet.champs.push(("pays", None, Some(c.pays.clone())));
    au_cabinet
        .champs
        .push(("devise_tenue", None, Some(c.devise.clone())));
    let mut au_dossier = Trace::new(PorteeAudit::Dossier(dossier.id), "dossier", id);
    au_dossier.champs.push(("nom", None, Some(nom.clone())));
    au_dossier.donnees = donnees;

    let vue = VueDossier {
        id: dossier.id.to_string(),
        nom,
        pays: c.pays.clone(),
        devise: c.devise.clone(),
        decimales: c.decimales,
        fuseau: c.fuseau.clone(),
        langue: c.langue.clone(),
        forme_juridique: c.forme_juridique.clone(),
        identifiants_legaux: Some(
            demande
                .identifiants()
                .iter()
                .map(|i| VueIdentifiant {
                    type_identifiant: i.type_identifiant.clone(),
                    valeur: i.valeur.clone(),
                })
                .collect(),
        ),
    };
    Ok(Resultat {
        reponse: (StatusCode::CREATED, Json(vue)).into_response(),
        traces: vec![au_cabinet, au_dossier],
    })
}

/// Corps de `POST …/dossiers/{dossier}/contexte` : chaque champ absent est conservé ;
/// `"forme_juridique": null` l'efface ; `identifiants_legaux`, s'il est présent, **remplace** la
/// liste entière (`[]` la vide).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpsModification {
    pays: Option<String>,
    devise: Option<String>,
    fuseau: Option<String>,
    langue: Option<String>,
    #[serde(default)]
    forme_juridique: ChampTexte,
    identifiants_legaux: Option<Vec<super::referentiel::CorpsIdentifiant>>,
}

/// Champ texte d'une modification : absent (conservé), `null` (effacé) ou une valeur.
#[derive(Debug, Default)]
enum ChampTexte {
    #[default]
    Absent,
    Efface,
    Valeur(String),
}

impl<'de> Deserialize<'de> for ChampTexte {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Option::<String>::deserialize(d)? {
            None => Self::Efface,
            Some(v) => Self::Valeur(v),
        })
    }
}

/// Défense en profondeur : appartenance **active** lue en base, de rôle `administrateur`
/// (Cedar décide aussi). Absente ou suspendue : 404 ; autre rôle : 403.
async fn exiger_administrateur(tx: &mut TransactionTenant) -> Result<(), ErreurApi> {
    let appartenance = lire_appartenance(tx)
        .await?
        .filter(|a| a.actif)
        .ok_or(ErreurApi::Introuvable)?;
    if appartenance.role == authz::Role::Administrateur.nom() {
        Ok(())
    } else {
        Err(ErreurApi::Interdit)
    }
}

/// Identifiants triés par type, pour les comparer sans égard à l'ordre.
fn tries(mut v: Vec<IdentifiantLegal>) -> Vec<IdentifiantLegal> {
    v.sort_by(|a, b| a.type_identifiant.cmp(&b.type_identifiant));
    v
}

/// Types d'identifiants d'une liste, pour l'audit (les valeurs n'y sont jamais recopiées).
fn types_de(v: &[IdentifiantLegal]) -> String {
    v.iter()
        .map(|i| i.type_identifiant.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// Trace d'audit de la modification : un champ par valeur modifiée (avant, après), au journal du
/// dossier ; les identifiants légaux n'y figurent que par leurs types (jamais leurs valeurs).
fn trace_de_modification(
    dossier: crate::base::IdDossier,
    (actuel, nouveau): (&ContexteDossier, &ContexteDossier),
    identifiants: Option<&(String, String)>,
) -> Trace {
    let mut trace = Trace::new(PorteeAudit::Dossier(dossier), "dossier", dossier.uuid());
    let mut champ = |nom: &'static str, avant: Option<&str>, apres: Option<&str>| {
        if avant != apres {
            trace
                .champs
                .push((nom, avant.map(str::to_owned), apres.map(str::to_owned)));
        }
    };
    champ("pays", Some(&actuel.pays), Some(&nouveau.pays));
    champ("devise_tenue", Some(&actuel.devise), Some(&nouveau.devise));
    champ(
        "decimales_tenue",
        Some(&actuel.decimales.to_string()),
        Some(&nouveau.decimales.to_string()),
    );
    champ("fuseau", Some(&actuel.fuseau), Some(&nouveau.fuseau));
    champ("langue", Some(&actuel.langue), Some(&nouveau.langue));
    champ(
        "forme_juridique",
        actuel.forme_juridique.as_deref(),
        nouveau.forme_juridique.as_deref(),
    );
    if let Some((avant, apres)) = identifiants {
        champ("identifiants_legaux", Some(avant), Some(apres));
    }
    trace
}

/// Modification du contexte du dossier (ADR 0027). Fuseau, langue et forme juridique changent à
/// tout moment. Pays et devise de tenue (donc échelle) exigent une **session fraîche** et un
/// dossier **sans écriture validée ni brouillon** ; le remplacement des identifiants légaux
/// exige seulement l'absence d'écriture validée. Contrôles sous le verrou de la tête de chaîne
/// (qui sérialise les validations) puis sous le verrou de la ligne du dossier (`FOR NO KEY
/// UPDATE`, qui attend les saisies de brouillon en cours : elles tiennent `FOR SHARE` sur cette
/// ligne jusqu'à leur validation en base). Ordre : tête, ligne du dossier, audit (pris par le
/// pipeline). Un changement de pays exige que les identifiants légaux (fournis ou existants)
/// restent admis par le nouveau pays. Une trace d'audit par champ modifié, au journal du dossier.
#[allow(clippy::too_many_lines)]
async fn modifier(
    tx: &mut TransactionTenant,
    garde: &Garde,
    requete: &Requete<'_>,
) -> Result<Resultat, ErreurApi> {
    exiger_administrateur(tx).await?;
    let id = tx.contexte().id_dossier().ok_or(ErreurApi::Introuvable)?;
    let visible = lire_dossier_visible(tx, id)
        .await?
        .ok_or(ErreurApi::Introuvable)?;
    let autorise = garde.dossier_du_chemin(visible)?;
    let Json(c) = Json::<CorpsModification>::from_bytes(requete.corps)
        .map_err(|_| ErreurApi::RequeteInvalide)?;
    let modification = ModificationContexte {
        pays: c.pays.as_deref(),
        devise: c.devise.as_deref(),
        fuseau: c.fuseau.as_deref(),
        langue: c.langue.as_deref(),
        forme_juridique: match &c.forme_juridique {
            ChampTexte::Absent => None,
            ChampTexte::Efface => Some(None),
            ChampTexte::Valeur(v) => Some(Some(v.as_str())),
        },
    };
    if modification.est_vide() && c.identifiants_legaux.is_none() {
        return Err(ErreurApi::RequeteInvalide);
    }
    let pack = PackPays::officiel().map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?;
    // Verrous : tête de chaîne (les validations en cours se terminent, les suivantes attendent),
    // puis ligne du dossier (les saisies en cours se terminent) ; tout est relu après.
    let genese = integrite::genese(
        &integrite::IdDossier::parse(&id.uuid().to_string())
            .map_err(|_| ErreurApi::Interne(CategorieErreur::Donnee))?,
    );
    crate::base::validation::verrouiller_tete(tx, &genese).await?;
    let actuel = verrouiller_dossier_pour_modification(tx).await?;
    let nouveau = modifier_le_contexte(pack, &actuel, &modification)
        .map_err(super::referentiel::refus_contexte)?;
    let existants = tries(lire_identifiants_legaux(tx).await?);
    let demandes = c.identifiants_legaux.map(|v| {
        tries(
            v.into_iter()
                .map(|i| IdentifiantLegal {
                    type_identifiant: i.type_identifiant,
                    valeur: i.valeur,
                })
                .collect(),
        )
    });
    let identifiants_changent = demandes.as_ref().is_some_and(|d| *d != existants);
    if nouveau == actuel && !identifiants_changent {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::AucunChangement,
            nombre: None,
        });
    }
    let sensible = actuel.change_pays_ou_devise(&nouveau);
    if sensible && !garde.session_fraiche() {
        return Err(ErreurApi::Interdit);
    }
    if sensible || identifiants_changent {
        let (validee, brouillon) = ecritures_du_dossier(tx).await?;
        let conflit = |motif| ErreurApi::Conflit {
            motif,
            nombre: None,
        };
        if validee {
            return Err(conflit(MotifConflit::DossierDejaEcritures));
        }
        if sensible && brouillon {
            return Err(conflit(MotifConflit::DossierBrouillardNonVide));
        }
        // Un arrêté de caisse fige aussi la devise (L3-21 ; seconde barrière : déclencheur
        // `dossier_devise_tresorerie`, migration 0540).
        if sensible && crate::base::tresorerie::a_des_arretes(tx).await? {
            return Err(conflit(MotifConflit::DossierArretesDeCaisse));
        }
    }
    // Le pays d'un dossier qui a une version de paramètres fiscaux propre ne change plus : la
    // table, en ajout seul, deviendrait orpheline (ADR 0030).
    if nouveau.pays != actuel.pays && a_des_parametres_fiscaux_propres(tx).await? {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::DossierParametresFiscauxPropres,
            nombre: None,
        });
    }
    // Ni le pays d'un dossier qui a des pièces : leur rétention est celle du pays au dépôt.
    if nouveau.pays != actuel.pays && a_des_pieces(tx).await? {
        return Err(ErreurApi::Conflit {
            motif: MotifConflit::DossierPiecesPresentes,
            nombre: None,
        });
    }
    // Liste finale : celle qui est fournie, sinon l'existante ; elle doit rester admise par le
    // pays (nouveau ou inchangé).
    let finaux = demandes.clone().unwrap_or_else(|| existants.clone());
    if identifiants_changent || nouveau.pays != actuel.pays {
        controler_identifiants(pack, &nouveau.pays, &finaux)
            .map_err(super::referentiel::refus_contexte)?;
    }
    if nouveau != actuel {
        modifier_contexte(tx, &nouveau).await?;
    }
    if identifiants_changent {
        remplacer_identifiants_legaux(tx, &finaux).await?;
    }

    let trace = trace_de_modification(
        autorise.id(),
        (&actuel, &nouveau),
        identifiants_changent
            .then(|| (types_de(&existants), types_de(&finaux)))
            .as_ref(),
    );
    let vue = VueDossier::depuis(
        autorise.id().to_string(),
        autorise.nom(),
        &nouveau,
        Some(finaux),
    );
    Ok(Resultat {
        reponse: (StatusCode::OK, Json(vue)).into_response(),
        traces: vec![trace],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_de_dossier() {
        assert_eq!(nom_dossier("SARL Exemple"), Ok("SARL Exemple".to_owned()));
        assert!(nom_dossier(&"é".repeat(200)).is_ok());
        for invalide in [
            String::new(),
            "   ".to_owned(),
            "a\u{7}b".to_owned(),
            "a".repeat(201),
        ] {
            assert_eq!(
                nom_dossier(&invalide),
                Err(ErreurApi::RequeteInvalide),
                "{invalide}"
            );
        }
    }
}
