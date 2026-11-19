//! Entrée canonique du journal d'audit, format **audit v1** (ADR 0014).
//!
//! Le format est décrit dans `docs/integrite/audit-v1.md`. Il est **figé** : toute modification
//! produit une version 2 (vecteurs de test dans `tests/vecteurs/audit-v1/`). Il suit les règles
//! communes de `format-v1.md` §1 : texte UTF-8, une donnée par ligne (`cle=valeur`, fin de ligne
//! `\n`, la dernière comprise), ordre des clés fixe, textes sans caractère de contrôle repris
//! tels quels. La lecture est **stricte** : un texte n'est accepté que s'il est exactement la
//! sérialisation de l'entrée qu'il décrit (menace F7.3).

use std::fmt::{self, Write as _};

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use crate::chaine::Empreinte;

/// Première ligne d'une entrée. La version entre dans le contenu haché.
pub const ENTETE: &str = "audit v1";
/// Dernière ligne d'une entrée.
pub const FIN: &str = "fin";
/// Longueur maximale, en octets UTF-8, d'un texte (motif, valeur, avant, après, nom d'acteur) :
/// la plus longue ligne de texte (`valeur=`, 7 octets, et la fin de ligne) tient dans les
/// 1 024 octets d'une ligne d'export (`format-v1.md` §4).
pub const TEXTE_MAX: usize = 1000;
/// Longueur maximale d'un symbole.
pub const SYMBOLE_MAX: usize = 64;
/// Taille maximale d'une entrée canonique (comme une écriture, `format-v1.md` §4).
pub const ENTREE_MAX: usize = 4 * 1024 * 1024;
/// Valeur de `acteur=` pour une tâche de la file (ADR 0011, « Tables d'instance »).
pub const SYSTEME: &str = "systeme";

/// Erreur de construction ou de lecture d'une entrée. `ligne` compte à partir de 1 dans
/// l'entrée.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErreurAudit {
    /// UUID qui n'est pas sous sa forme canonique (8-4-4-4-12, hexadécimal minuscule).
    #[error("UUID invalide : forme 8-4-4-4-12 en hexadécimal minuscule attendue")]
    Uuid,
    /// Symbole vide, trop long ou hors de `A-Z a-z 0-9 . _ -`.
    #[error("symbole invalide : 1 à 64 caractères parmi A-Z a-z 0-9 . _ -")]
    Symbole,
    /// Texte trop long ou qui contient un caractère de contrôle.
    #[error("texte invalide : caractère de contrôle ou plus de {TEXTE_MAX} octets")]
    Texte,
    /// Motif vide ou fait d'espaces seulement.
    #[error("motif vide")]
    MotifVide,
    /// Rang nul : le premier rang d'une chaîne est 1.
    #[error("rang nul : le premier rang est 1")]
    RangNul,
    /// Instant hors UTC, plus fin que la microseconde, ou hors des années 0000 à 9999.
    #[error("instant invalide : UTC à la microseconde, années 0000 à 9999")]
    Instant,
    /// Décision sans politique déterminante : une autorisation de Cedar en a toujours une.
    #[error("décision sans politique déterminante")]
    SansPolitique,
    /// Politique, donnée ou champ en double.
    #[error("« {0} » en double")]
    Doublon(&'static str),
    /// Champ sans valeur avant ni après.
    #[error("champ sans valeur avant ni après")]
    ChampVide,
    /// Une ligne attendue manque ou ne porte pas la bonne clé.
    #[error("ligne {ligne} : « {attendu} » attendu")]
    Attendu {
        /// Ligne fautive.
        ligne: usize,
        /// Clé ou marqueur attendu.
        attendu: &'static str,
    },
    /// La valeur d'une clé est invalide.
    #[error("ligne {ligne} : valeur de « {cle} » invalide")]
    ValeurInvalide {
        /// Ligne fautive.
        ligne: usize,
        /// Clé concernée.
        cle: &'static str,
    },
    /// Texte après la ligne `fin`.
    #[error("ligne {ligne} : texte après « fin »")]
    TexteEnTrop {
        /// Première ligne en trop.
        ligne: usize,
    },
    /// Le texte décrit une entrée valide mais n'est pas sa forme canonique (politiques dans le
    /// désordre, rang avec un zéro de tête…).
    #[error("forme non canonique")]
    NonCanonique,
}

/// UUID sous sa forme canonique : 8-4-4-4-12 chiffres hexadécimaux minuscules.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid([u8; 16]);

impl Uuid {
    /// UUID de 16 octets.
    #[must_use]
    pub const fn depuis_octets(octets: [u8; 16]) -> Self {
        Self(octets)
    }

    /// Les 16 octets.
    #[must_use]
    pub const fn octets(&self) -> &[u8; 16] {
        &self.0
    }

    /// Lit un UUID sous sa forme canonique (36 caractères, minuscules, tirets).
    ///
    /// # Errors
    /// [`ErreurAudit::Uuid`].
    pub fn parse(texte: &str) -> Result<Self, ErreurAudit> {
        let octets = texte.as_bytes();
        if octets.len() != 36 {
            return Err(ErreurAudit::Uuid);
        }
        let mut chiffres = Vec::with_capacity(32);
        for (i, &b) in octets.iter().enumerate() {
            if matches!(i, 8 | 13 | 18 | 23) {
                if b != b'-' {
                    return Err(ErreurAudit::Uuid);
                }
                continue;
            }
            chiffres.push(match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                _ => return Err(ErreurAudit::Uuid),
            });
        }
        let mut sortie = [0u8; 16];
        for (i, [fort, faible]) in chiffres.as_chunks::<2>().0.iter().enumerate() {
            sortie[i] = fort << 4 | faible;
        }
        Ok(Self(sortie))
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Uuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({self})")
    }
}

/// Symbole : 1 à 64 caractères parmi `A-Z a-z 0-9 . _ -`. Rôle, action Cedar, politique, genre
/// de ressource, nom de donnée ou de champ.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbole(String);

impl Symbole {
    /// Valide un symbole.
    ///
    /// # Errors
    /// [`ErreurAudit::Symbole`].
    pub fn parse(texte: &str) -> Result<Self, ErreurAudit> {
        let valide = (1..=SYMBOLE_MAX).contains(&texte.len())
            && texte
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if valide {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurAudit::Symbole)
        }
    }

    /// Le symbole.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Texte repris tel quel : au plus [`TEXTE_MAX`] octets, sans caractère de contrôle, sans
/// normalisation Unicode ni retrait d'espaces. Il peut être vide (une valeur effacée).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Texte(String);

impl Texte {
    /// Valide un texte.
    ///
    /// # Errors
    /// [`ErreurAudit::Texte`].
    pub fn parse(texte: &str) -> Result<Self, ErreurAudit> {
        if texte.len() <= TEXTE_MAX && !texte.chars().any(char::is_control) {
            Ok(Self(texte.to_owned()))
        } else {
            Err(ErreurAudit::Texte)
        }
    }

    /// Le texte.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Texte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Portée d'une chaîne d'audit (ADR 0014) : le cabinet, ou un dossier de ce cabinet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Portee {
    /// Chaîne du cabinet : administration, droits, création des dossiers.
    Cabinet {
        /// Le cabinet.
        cabinet: Uuid,
    },
    /// Chaîne d'un dossier.
    Dossier {
        /// Le cabinet qui tient le dossier.
        cabinet: Uuid,
        /// Le dossier.
        dossier: Uuid,
    },
}

impl Portee {
    /// Valeur de `portee=` : `cabinet` ou `dossier`.
    #[must_use]
    pub const fn nom(&self) -> &'static str {
        match self {
            Self::Cabinet { .. } => "cabinet",
            Self::Dossier { .. } => "dossier",
        }
    }

    /// Identifiant de la chaîne : le cabinet ou le dossier.
    #[must_use]
    pub const fn identifiant(&self) -> Uuid {
        match self {
            Self::Cabinet { cabinet } => *cabinet,
            Self::Dossier { dossier, .. } => *dossier,
        }
    }

    /// Le cabinet.
    #[must_use]
    pub const fn cabinet(&self) -> Uuid {
        match self {
            Self::Cabinet { cabinet } | Self::Dossier { cabinet, .. } => *cabinet,
        }
    }

    /// Le dossier, pour une portée de dossier.
    #[must_use]
    pub const fn dossier(&self) -> Option<Uuid> {
        match self {
            Self::Cabinet { .. } => None,
            Self::Dossier { dossier, .. } => Some(*dossier),
        }
    }
}

/// Décision de Cedar qui a autorisé l'action d'un utilisateur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionCedar {
    politiques: Vec<Symbole>,
    version: Empreinte,
    reauthentification: bool,
}

impl DecisionCedar {
    /// Politiques déterminantes (`@id`, une au moins, triées à la construction), version du jeu
    /// de politiques et ré-authentification (session « fraîche », ADR 0012).
    ///
    /// # Errors
    /// [`ErreurAudit::SansPolitique`], [`ErreurAudit::Doublon`].
    pub fn new(
        mut politiques: Vec<Symbole>,
        version: Empreinte,
        reauthentification: bool,
    ) -> Result<Self, ErreurAudit> {
        if politiques.is_empty() {
            return Err(ErreurAudit::SansPolitique);
        }
        politiques.sort();
        if politiques.windows(2).any(|p| p[0] == p[1]) {
            return Err(ErreurAudit::Doublon("politique"));
        }
        Ok(Self {
            politiques,
            version,
            reauthentification,
        })
    }

    /// Politiques déterminantes, dans l'ordre des octets.
    #[must_use]
    pub fn politiques(&self) -> &[Symbole] {
        &self.politiques
    }

    /// Version du jeu de politiques.
    #[must_use]
    pub fn version(&self) -> &Empreinte {
        &self.version
    }

    /// Vrai si la session venait d'être ré-authentifiée.
    #[must_use]
    pub fn reauthentification(&self) -> bool {
        self.reauthentification
    }
}

/// Qui agit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acteur {
    /// Une tâche de la file, sans décision de Cedar.
    Systeme,
    /// Un utilisateur, désigné par son UUID (son nom est résolu à l'affichage).
    Utilisateur {
        /// L'utilisateur.
        id: Uuid,
        /// Son rôle dans le cabinet.
        role: Symbole,
        /// Identifiant public de sa session, jamais son jeton.
        session: Uuid,
        /// La décision de Cedar.
        decision: DecisionCedar,
    },
}

/// Ressource visée : `genre:uuid` (`dossier:…`, `cabinet:…`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ressource {
    /// Genre de la ressource.
    pub genre: Symbole,
    /// Identifiant.
    pub id: Uuid,
}

impl fmt::Display for Ressource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.genre, self.id)
    }
}

impl Ressource {
    fn parse(texte: &str) -> Result<Self, ErreurAudit> {
        let (genre, id) = texte.split_once(':').ok_or(ErreurAudit::Symbole)?;
        Ok(Self {
            genre: Symbole::parse(genre)?,
            id: Uuid::parse(id)?,
        })
    }
}

/// Champ modifié : valeur avant (absente pour une création), valeur après (absente pour une
/// suppression).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Champ {
    nom: Symbole,
    avant: Option<Texte>,
    apres: Option<Texte>,
}

impl Champ {
    /// Un champ modifié.
    ///
    /// # Errors
    /// [`ErreurAudit::ChampVide`] sans valeur avant ni après.
    pub fn new(
        nom: Symbole,
        avant: Option<Texte>,
        apres: Option<Texte>,
    ) -> Result<Self, ErreurAudit> {
        if avant.is_none() && apres.is_none() {
            return Err(ErreurAudit::ChampVide);
        }
        Ok(Self { nom, avant, apres })
    }

    /// Nom du champ.
    #[must_use]
    pub fn nom(&self) -> &Symbole {
        &self.nom
    }

    /// Valeur avant.
    #[must_use]
    pub fn avant(&self) -> Option<&Texte> {
        self.avant.as_ref()
    }

    /// Valeur après.
    #[must_use]
    pub fn apres(&self) -> Option<&Texte> {
        self.apres.as_ref()
    }
}

/// Ce qui s'est passé, indépendamment de sa place dans une chaîne : acteur, action, ressource,
/// motif, données citées (tête et nombre exportés, numéro et empreinte d'une écriture
/// validée…) et champs modifiés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evenement {
    acteur: Acteur,
    action: Symbole,
    ressource: Ressource,
    motif: Option<Texte>,
    donnees: Vec<(Symbole, Texte)>,
    champs: Vec<Champ>,
}

impl Evenement {
    /// Un événement sans motif, donnée ni champ.
    #[must_use]
    pub fn new(acteur: Acteur, action: Symbole, ressource: Ressource) -> Self {
        Self {
            acteur,
            action,
            ressource,
            motif: None,
            donnees: Vec::new(),
            champs: Vec::new(),
        }
    }

    /// Ajoute le motif de l'action (déverrouillage, désactivation de la séparation des tâches).
    ///
    /// # Errors
    /// [`ErreurAudit::MotifVide`].
    pub fn avec_motif(mut self, motif: Texte) -> Result<Self, ErreurAudit> {
        if motif.as_str().trim().is_empty() {
            return Err(ErreurAudit::MotifVide);
        }
        self.motif = Some(motif);
        Ok(self)
    }

    /// Ajoute une donnée citée (fait de l'action, pas une modification).
    ///
    /// # Errors
    /// [`ErreurAudit::Doublon`] si le nom est déjà cité.
    pub fn avec_donnee(mut self, nom: Symbole, valeur: Texte) -> Result<Self, ErreurAudit> {
        if self.donnees.iter().any(|(n, _)| *n == nom) {
            return Err(ErreurAudit::Doublon("donnee"));
        }
        self.donnees.push((nom, valeur));
        Ok(self)
    }

    /// Ajoute un champ modifié.
    ///
    /// # Errors
    /// [`ErreurAudit::Doublon`] si le champ figure déjà.
    pub fn avec_champ(mut self, champ: Champ) -> Result<Self, ErreurAudit> {
        if self.champs.iter().any(|c| c.nom == champ.nom) {
            return Err(ErreurAudit::Doublon("champ"));
        }
        self.champs.push(champ);
        Ok(self)
    }

    /// L'acteur.
    #[must_use]
    pub fn acteur(&self) -> &Acteur {
        &self.acteur
    }

    /// L'action (identifiant d'action Cedar).
    #[must_use]
    pub fn action(&self) -> &Symbole {
        &self.action
    }

    /// La ressource.
    #[must_use]
    pub fn ressource(&self) -> &Ressource {
        &self.ressource
    }

    /// Le motif.
    #[must_use]
    pub fn motif(&self) -> Option<&Texte> {
        self.motif.as_ref()
    }

    /// Les données citées, dans l'ordre.
    #[must_use]
    pub fn donnees(&self) -> &[(Symbole, Texte)] {
        &self.donnees
    }

    /// Les champs modifiés, dans l'ordre.
    #[must_use]
    pub fn champs(&self) -> &[Champ] {
        &self.champs
    }
}

/// Entrée du journal d'audit : un événement à sa place dans la chaîne d'une portée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntreeAudit {
    portee: Portee,
    rang: u64,
    instant: OffsetDateTime,
    evenement: Evenement,
}

impl EntreeAudit {
    /// Entrée de rang `rang` (1 pour la première) de la chaîne de `portee`, à `instant` (UTC,
    /// à la microseconde, horloge de `api`).
    ///
    /// # Errors
    /// [`ErreurAudit::RangNul`], [`ErreurAudit::Instant`].
    pub fn nouvelle(
        portee: Portee,
        rang: u64,
        instant: OffsetDateTime,
        evenement: Evenement,
    ) -> Result<Self, ErreurAudit> {
        if rang == 0 {
            return Err(ErreurAudit::RangNul);
        }
        let valide = instant.offset() == UtcOffset::UTC
            && instant.nanosecond().is_multiple_of(1000)
            && (0..=9999).contains(&instant.year());
        if !valide {
            return Err(ErreurAudit::Instant);
        }
        Ok(Self {
            portee,
            rang,
            instant,
            evenement,
        })
    }

    /// La portée.
    #[must_use]
    pub fn portee(&self) -> &Portee {
        &self.portee
    }

    /// Le rang dans la chaîne de la portée.
    #[must_use]
    pub fn rang(&self) -> u64 {
        self.rang
    }

    /// L'instant (UTC).
    #[must_use]
    pub fn instant(&self) -> OffsetDateTime {
        self.instant
    }

    /// L'événement.
    #[must_use]
    pub fn evenement(&self) -> &Evenement {
        &self.evenement
    }

    /// Sérialisation canonique : les octets hachés.
    #[must_use]
    pub fn serialiser(&self) -> String {
        let mut s = String::with_capacity(512);
        let mut cle = |cle: &str, valeur: &dyn fmt::Display| {
            // L'écriture dans une `String` ne peut pas échouer.
            let _ = writeln!(s, "{cle}={valeur}");
        };
        cle("portee", &self.portee.nom());
        cle("cabinet", &self.portee.cabinet());
        if let Some(dossier) = self.portee.dossier() {
            cle("dossier", &dossier);
        }
        cle("rang", &self.rang);
        cle("instant", &InstantIso(self.instant));
        let e = &self.evenement;
        match &e.acteur {
            Acteur::Systeme => cle("acteur", &SYSTEME),
            Acteur::Utilisateur {
                id, role, session, ..
            } => {
                cle("acteur", id);
                cle("role", role);
                cle("session", session);
            }
        }
        cle("action", &e.action);
        cle("ressource", &e.ressource);
        if let Acteur::Utilisateur { decision, .. } = &e.acteur {
            for politique in &decision.politiques {
                cle("politique", politique);
            }
            cle("politiques", &decision.version);
            cle(
                "reauthentification",
                &if decision.reauthentification {
                    "oui"
                } else {
                    "non"
                },
            );
        }
        if let Some(motif) = &e.motif {
            cle("motif", motif);
        }
        for (nom, valeur) in &e.donnees {
            cle("donnee", nom);
            cle("valeur", valeur);
        }
        for champ in &e.champs {
            cle("champ", &champ.nom);
            if let Some(avant) = &champ.avant {
                cle("avant", avant);
            }
            if let Some(apres) = &champ.apres {
                cle("apres", apres);
            }
        }
        format!("{ENTETE}\n{s}{FIN}\n")
    }

    /// Lit une entrée et vérifie que le texte est exactement sa forme canonique.
    ///
    /// # Errors
    /// [`ErreurAudit`] : structure, valeur ou forme non canonique.
    pub fn lire(texte: &str) -> Result<Self, ErreurAudit> {
        if texte.len() > ENTREE_MAX {
            return Err(ErreurAudit::Attendu {
                ligne: 0,
                attendu: FIN,
            });
        }
        let entree = Lecteur::nouveau(texte).entree()?;
        if entree.serialiser() == texte {
            Ok(entree)
        } else {
            Err(ErreurAudit::NonCanonique)
        }
    }
}

/// Forme canonique d'un instant, telle que l'écrit la ligne `instant=` :
/// `AAAA-MM-JJTHH:MM:SS.ffffffZ` (UTC, six décimales).
#[must_use]
pub fn formater_instant(instant: OffsetDateTime) -> String {
    InstantIso(instant.to_offset(UtcOffset::UTC)).to_string()
}

/// Instant `AAAA-MM-JJTHH:MM:SS.ffffffZ` (UTC, six décimales).
struct InstantIso(OffsetDateTime);

impl fmt::Display for InstantIso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let i = self.0;
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z",
            i.year(),
            u8::from(i.month()),
            i.day(),
            i.hour(),
            i.minute(),
            i.second(),
            i.microsecond()
        )
    }
}

fn lire_instant(texte: &str) -> Option<OffsetDateTime> {
    let o = texte.as_bytes();
    let forme = o.len() == 27
        && o[4] == b'-'
        && o[7] == b'-'
        && o[10] == b'T'
        && o[13] == b':'
        && o[16] == b':'
        && o[19] == b'.'
        && o[26] == b'Z'
        && o.iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19 | 26) || b.is_ascii_digit());
    if !forme {
        return None;
    }
    let n = |a: usize, b: usize| texte.get(a..b)?.parse::<u32>().ok();
    let annee = i32::try_from(n(0, 4)?).ok()?;
    let date = Date::from_calendar_date(
        annee,
        Month::try_from(u8::try_from(n(5, 7)?).ok()?).ok()?,
        u8::try_from(n(8, 10)?).ok()?,
    )
    .ok()?;
    let heure = Time::from_hms_micro(
        u8::try_from(n(11, 13)?).ok()?,
        u8::try_from(n(14, 16)?).ok()?,
        u8::try_from(n(17, 19)?).ok()?,
        n(20, 26)?,
    )
    .ok()?;
    Some(PrimitiveDateTime::new(date, heure).assume_utc())
}

/// Entier décimal strictement positif, sans zéro de tête ni signe.
fn lire_rang(texte: &str) -> Option<u64> {
    let canonique =
        !texte.is_empty() && texte.bytes().all(|b| b.is_ascii_digit()) && !texte.starts_with('0');
    if canonique { texte.parse().ok() } else { None }
}

/// Lecture séquentielle des lignes d'une entrée.
struct Lecteur<'a> {
    lignes: std::iter::Peekable<std::iter::Enumerate<std::str::Split<'a, char>>>,
}

impl<'a> Lecteur<'a> {
    fn nouveau(texte: &'a str) -> Self {
        // `split('\n')` sur un texte terminé par `\n` donne une dernière pièce vide, contrôlée
        // en fin de lecture.
        Self {
            lignes: texte.split('\n').enumerate().peekable(),
        }
    }

    fn suivante(&mut self) -> Option<(usize, &'a str)> {
        self.lignes.next().map(|(i, l)| (i + 1, l))
    }

    fn rang_courant(&mut self) -> usize {
        self.lignes.peek().map_or(0, |(i, _)| i + 1)
    }

    fn prochaine_cle(&mut self) -> Option<&'a str> {
        self.lignes
            .peek()
            .and_then(|(_, l)| l.split_once('='))
            .map(|(k, _)| k)
    }

    fn cle(&mut self, cle: &'static str) -> Result<(usize, &'a str), ErreurAudit> {
        let rang = self.rang_courant();
        match self.suivante() {
            Some((ligne, l)) => match l.split_once('=') {
                Some((k, v)) if k == cle => Ok((ligne, v)),
                _ => Err(ErreurAudit::Attendu {
                    ligne,
                    attendu: cle,
                }),
            },
            None => Err(ErreurAudit::Attendu {
                ligne: rang,
                attendu: cle,
            }),
        }
    }

    fn valeur<T>(
        &mut self,
        cle: &'static str,
        analyse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<T, ErreurAudit> {
        let (ligne, v) = self.cle(cle)?;
        analyse(v).ok_or(ErreurAudit::ValeurInvalide { ligne, cle })
    }

    fn valeur_optionnelle<T>(
        &mut self,
        cle: &'static str,
        analyse: impl FnOnce(&str) -> Option<T>,
    ) -> Result<Option<T>, ErreurAudit> {
        if self.prochaine_cle() == Some(cle) {
            self.valeur(cle, analyse).map(Some)
        } else {
            Ok(None)
        }
    }

    fn entree(mut self) -> Result<EntreeAudit, ErreurAudit> {
        match self.suivante() {
            Some((_, ENTETE)) => {}
            Some((ligne, _)) => {
                return Err(ErreurAudit::Attendu {
                    ligne,
                    attendu: ENTETE,
                });
            }
            None => {
                return Err(ErreurAudit::Attendu {
                    ligne: 0,
                    attendu: ENTETE,
                });
            }
        }
        let uuid = |v: &str| Uuid::parse(v).ok();
        let symbole = |v: &str| Symbole::parse(v).ok();

        let dossier = self.valeur("portee", |v| match v {
            "cabinet" => Some(false),
            "dossier" => Some(true),
            _ => None,
        })?;
        let cabinet = self.valeur("cabinet", uuid)?;
        let portee = if dossier {
            Portee::Dossier {
                cabinet,
                dossier: self.valeur("dossier", uuid)?,
            }
        } else {
            Portee::Cabinet { cabinet }
        };
        let rang = self.valeur("rang", lire_rang)?;
        let (ligne_instant, instant) = self.cle("instant")?;
        let instant = lire_instant(instant).ok_or(ErreurAudit::ValeurInvalide {
            ligne: ligne_instant,
            cle: "instant",
        })?;
        let acteur = self.valeur("acteur", |v| {
            if v == SYSTEME {
                Some(None)
            } else {
                Uuid::parse(v).ok().map(Some)
            }
        })?;
        let utilisateur = match acteur {
            Some(id) => Some((
                id,
                self.valeur("role", symbole)?,
                self.valeur("session", uuid)?,
            )),
            None => None,
        };
        let action = self.valeur("action", symbole)?;
        let ressource = self.valeur("ressource", |v| Ressource::parse(v).ok())?;
        let acteur = match utilisateur {
            None => Acteur::Systeme,
            Some((id, role, session)) => Acteur::Utilisateur {
                id,
                role,
                session,
                decision: self.decision()?,
            },
        };
        let evenement = self.details(Evenement::new(acteur, action, ressource))?;
        match self.suivante() {
            Some((_, FIN)) => {}
            Some((ligne, _)) => {
                return Err(ErreurAudit::Attendu {
                    ligne,
                    attendu: FIN,
                });
            }
            None => {
                return Err(ErreurAudit::Attendu {
                    ligne: 0,
                    attendu: FIN,
                });
            }
        }
        // Après `fin\n`, il ne reste que la pièce vide finale.
        match (self.suivante(), self.suivante()) {
            (Some((_, "")), None) => {}
            (Some((ligne, _)), _) => return Err(ErreurAudit::TexteEnTrop { ligne }),
            (None, _) => {
                return Err(ErreurAudit::Attendu {
                    ligne: 0,
                    attendu: FIN,
                });
            }
        }
        EntreeAudit::nouvelle(portee, rang, instant, evenement).map_err(|_| {
            ErreurAudit::ValeurInvalide {
                ligne: ligne_instant,
                cle: "instant",
            }
        })
    }

    /// Décision de Cedar d'un acteur utilisateur.
    fn decision(&mut self) -> Result<DecisionCedar, ErreurAudit> {
        let symbole = |v: &str| Symbole::parse(v).ok();
        let rang_politiques = self.rang_courant();
        let mut politiques = Vec::new();
        while let Some(p) = self.valeur_optionnelle("politique", symbole)? {
            politiques.push(p);
        }
        let version = self.valeur("politiques", |v| Empreinte::parse(v).ok())?;
        let reauthentification = self.valeur("reauthentification", |v| match v {
            "oui" => Some(true),
            "non" => Some(false),
            _ => None,
        })?;
        DecisionCedar::new(politiques, version, reauthentification).map_err(|e| match e {
            ErreurAudit::SansPolitique => ErreurAudit::Attendu {
                ligne: rang_politiques,
                attendu: "politique",
            },
            _ => ErreurAudit::NonCanonique,
        })
    }

    /// Motif, données citées et champs modifiés.
    fn details(&mut self, mut evenement: Evenement) -> Result<Evenement, ErreurAudit> {
        let symbole = |v: &str| Symbole::parse(v).ok();
        let texte = |v: &str| Texte::parse(v).ok();
        if let Some((ligne, motif)) = self.valeur_optionnelle_brute("motif")? {
            evenement = Texte::parse(motif)
                .and_then(|m| evenement.avec_motif(m))
                .map_err(|_| ErreurAudit::ValeurInvalide {
                    ligne,
                    cle: "motif",
                })?;
        }
        while self.prochaine_cle() == Some("donnee") {
            let ligne = self.rang_courant();
            let nom = self.valeur("donnee", symbole)?;
            let valeur = self.valeur("valeur", texte)?;
            evenement =
                evenement
                    .avec_donnee(nom, valeur)
                    .map_err(|_| ErreurAudit::ValeurInvalide {
                        ligne,
                        cle: "donnee",
                    })?;
        }
        while self.prochaine_cle() == Some("champ") {
            let ligne = self.rang_courant();
            let nom = self.valeur("champ", symbole)?;
            let avant = self.valeur_optionnelle("avant", texte)?;
            let apres = self.valeur_optionnelle("apres", texte)?;
            evenement = Champ::new(nom, avant, apres)
                .and_then(|c| evenement.avec_champ(c))
                .map_err(|_| ErreurAudit::ValeurInvalide {
                    ligne,
                    cle: "champ",
                })?;
        }
        Ok(evenement)
    }

    fn valeur_optionnelle_brute(
        &mut self,
        cle: &'static str,
    ) -> Result<Option<(usize, &'a str)>, ErreurAudit> {
        if self.prochaine_cle() == Some(cle) {
            self.cle(cle).map(Some)
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn uuid(n: u8) -> Uuid {
        Uuid::depuis_octets([n; 16])
    }

    fn sym(s: &str) -> Symbole {
        Symbole::parse(s).unwrap()
    }

    fn txt(s: &str) -> Texte {
        Texte::parse(s).unwrap()
    }

    fn exemple() -> EntreeAudit {
        let decision = DecisionCedar::new(
            vec![sym("s20-creer-dossier"), sym("adr0016-audit-cabinet")],
            Empreinte::calculer(b"politiques"),
            false,
        )
        .unwrap();
        let acteur = Acteur::Utilisateur {
            id: uuid(0xaa),
            role: sym("valideur"),
            session: uuid(0x55),
            decision,
        };
        let evenement = Evenement::new(
            acteur,
            sym("CreerDossier"),
            Ressource {
                genre: sym("dossier"),
                id: uuid(0x11),
            },
        )
        .avec_motif(txt("Ouverture = demande du client"))
        .unwrap()
        .avec_donnee(sym("tete"), txt("x"))
        .unwrap()
        .avec_champ(Champ::new(sym("nom"), None, Some(txt("SARL Exemple"))).unwrap())
        .unwrap()
        .avec_champ(Champ::new(sym("note"), Some(txt("")), Some(txt(" b "))).unwrap())
        .unwrap();
        EntreeAudit::nouvelle(
            Portee::Dossier {
                cabinet: uuid(0x01),
                dossier: uuid(0x11),
            },
            12,
            time::macros::datetime!(2026-11-18 09:14:03.12 UTC),
            evenement,
        )
        .unwrap()
    }

    #[test]
    fn aller_retour_et_forme() {
        let e = exemple();
        let texte = e.serialiser();
        assert_eq!(EntreeAudit::lire(&texte), Ok(e));
        assert!(texte.starts_with(
            "audit v1\nportee=dossier\ncabinet=01010101-0101-0101-0101-010101010101\n\
             dossier=11111111-1111-1111-1111-111111111111\nrang=12\n\
             instant=2026-11-18T09:14:03.120000Z\nacteur=aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\n\
             role=valideur\n"
        ));
        assert!(texte.contains("politique=adr0016-audit-cabinet\npolitique=s20-creer-dossier\n"));
        assert!(texte.ends_with("champ=note\navant=\napres= b \nfin\n"));
    }

    #[test]
    fn uuid_canonique() {
        let u = Uuid::parse("0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f").unwrap();
        assert_eq!(u.to_string(), "0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f");
        for invalide in [
            "0193E0C2-7d1a-4c4e-9f00-1a2b3c4d5e6f",
            "0193e0c27d1a4c4e9f001a2b3c4d5e6f",
            "{0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6f}",
            "0193e0c2-7d1a-4c4e-9f00-1a2b3c4d5e6g",
            "0193e0c2+7d1a-4c4e-9f00-1a2b3c4d5e6f",
        ] {
            assert_eq!(Uuid::parse(invalide), Err(ErreurAudit::Uuid), "{invalide}");
        }
    }

    #[test]
    fn refuse_les_formes_non_canoniques_et_invalides() {
        let texte = exemple().serialiser();
        let cas = [
            ("rang=12", "rang=012"),
            ("rang=12", "rang=+12"),
            ("rang=12", "rang=0"),
            (
                "instant=2026-11-18T09:14:03.120000Z",
                "instant=2026-11-18T09:14:03.12Z",
            ),
            (
                "instant=2026-11-18T09:14:03.120000Z",
                "instant=2026-11-18T09:14:03.120000+00:00",
            ),
            (
                "instant=2026-11-18T09:14:03.120000Z",
                "instant=2026-11-31T09:14:03.120000Z",
            ),
            (
                "politique=adr0016-audit-cabinet\npolitique=s20-creer-dossier\n",
                "politique=s20-creer-dossier\npolitique=adr0016-audit-cabinet\n",
            ),
            (
                "politique=adr0016-audit-cabinet\n",
                "politique=adr0016-audit-cabinet\npolitique=adr0016-audit-cabinet\n",
            ),
            (
                "politique=adr0016-audit-cabinet\npolitique=s20-creer-dossier\n",
                "",
            ),
            ("reauthentification=non", "reauthentification=Non"),
            ("portee=dossier", "portee=Dossier"),
            ("acteur=aaaaaaaa", "acteur=AAAAAAAA"),
            ("champ=note\navant=\napres= b \n", "champ=note\n"),
            ("champ=note", "champ=nom"),
            (
                "donnee=tete\nvaleur=x\n",
                "donnee=tete\nvaleur=x\ndonnee=tete\nvaleur=y\n",
            ),
            ("donnee=tete\nvaleur=x\n", "donnee=tete\n"),
            ("motif=Ouverture", "motif=\tOuverture"),
            ("motif=Ouverture = demande du client", "motif=  "),
            ("ressource=dossier:", "ressource=dossier;"),
            ("fin\n", "fin\nfin\n"),
            ("fin\n", "fin"),
            ("fin\n", ""),
            ("audit v1\n", "audit v2\n"),
            ("apres= b \n", "apres= b \r\n"),
        ];
        for (avant, apres) in cas {
            assert!(texte.contains(avant), "{avant}");
            let modifie = texte.replacen(avant, apres, 1);
            assert!(EntreeAudit::lire(&modifie).is_err(), "{avant} → {apres}");
        }
        // Portée de cabinet avec dossier : refusée.
        let cabinet = texte.replacen("portee=dossier", "portee=cabinet", 1);
        assert!(EntreeAudit::lire(&cabinet).is_err());
    }

    #[test]
    fn acteur_systeme_sans_decision() {
        let e = EntreeAudit::nouvelle(
            Portee::Cabinet { cabinet: uuid(1) },
            1,
            time::macros::datetime!(2026-01-01 0:00 UTC),
            Evenement::new(
                Acteur::Systeme,
                sym("Horodater"),
                Ressource {
                    genre: sym("cabinet"),
                    id: uuid(1),
                },
            ),
        )
        .unwrap();
        let texte = e.serialiser();
        assert_eq!(
            texte,
            "audit v1\nportee=cabinet\ncabinet=01010101-0101-0101-0101-010101010101\nrang=1\n\
             instant=2026-01-01T00:00:00.000000Z\nacteur=systeme\naction=Horodater\n\
             ressource=cabinet:01010101-0101-0101-0101-010101010101\nfin\n"
        );
        assert_eq!(EntreeAudit::lire(&texte), Ok(e));
        // Un acteur « systeme » n'a ni rôle, ni session, ni décision.
        let avec_role = texte.replacen("acteur=systeme\n", "acteur=systeme\nrole=valideur\n", 1);
        assert!(EntreeAudit::lire(&avec_role).is_err());
    }

    #[test]
    fn constructions_refusees() {
        let instant = time::macros::datetime!(2026-01-01 0:00 UTC);
        let ev = || {
            Evenement::new(
                Acteur::Systeme,
                sym("A"),
                Ressource {
                    genre: sym("cabinet"),
                    id: uuid(1),
                },
            )
        };
        let p = Portee::Cabinet { cabinet: uuid(1) };
        assert_eq!(
            EntreeAudit::nouvelle(p, 0, instant, ev()),
            Err(ErreurAudit::RangNul)
        );
        for invalide in [
            instant + time::Duration::nanoseconds(1),
            instant.to_offset(time::macros::offset!(+1)),
            time::macros::datetime!(-0001-01-01 0:00 UTC),
        ] {
            assert_eq!(
                EntreeAudit::nouvelle(p, 1, invalide, ev()),
                Err(ErreurAudit::Instant)
            );
        }
        assert_eq!(
            DecisionCedar::new(vec![], Empreinte::calculer(b""), true),
            Err(ErreurAudit::SansPolitique)
        );
        assert_eq!(
            DecisionCedar::new(vec![sym("a"), sym("a")], Empreinte::calculer(b""), true),
            Err(ErreurAudit::Doublon("politique"))
        );
        assert_eq!(
            Champ::new(sym("x"), None, None),
            Err(ErreurAudit::ChampVide)
        );
        assert_eq!(ev().avec_motif(txt(" ")), Err(ErreurAudit::MotifVide));
        assert!(Texte::parse(&"é".repeat(501)).is_err());
        assert!(Texte::parse(&"é".repeat(500)).is_ok());
        assert!(Texte::parse("a\u{7f}").is_err());
        assert!(Symbole::parse("a:b").is_err());
        assert!(Symbole::parse(&"a".repeat(65)).is_err());
    }

    mod proprietes {
        use super::*;
        use proptest::prelude::*;

        fn uuid_() -> impl Strategy<Value = Uuid> {
            any::<[u8; 16]>().prop_map(Uuid::depuis_octets)
        }

        fn symbole() -> impl Strategy<Value = Symbole> {
            "[A-Za-z0-9._-]{1,12}".prop_map(|s| Symbole::parse(&s).unwrap())
        }

        fn texte() -> impl Strategy<Value = Texte> {
            // Unicode quelconque, `=` et espaces compris, vide admis ; jamais de contrôle.
            "[^\\p{Cc}]{0,40}".prop_map(|t| Texte::parse(&t).unwrap())
        }

        fn instant() -> impl Strategy<Value = OffsetDateTime> {
            (0i64..253_402_300_799, 0u32..1_000_000).prop_map(|(s, us)| {
                OffsetDateTime::from_unix_timestamp(s - 62_167_219_200).unwrap()
                    + time::Duration::microseconds(i64::from(us))
            })
        }

        fn portee() -> impl Strategy<Value = Portee> {
            (uuid_(), proptest::option::of(uuid_())).prop_map(|(cabinet, dossier)| match dossier {
                None => Portee::Cabinet { cabinet },
                Some(dossier) => Portee::Dossier { cabinet, dossier },
            })
        }

        fn acteur() -> impl Strategy<Value = Acteur> {
            let decision = (
                proptest::collection::btree_set(symbole(), 1..4),
                any::<[u8; 32]>(),
                any::<bool>(),
            )
                .prop_map(|(p, v, r)| {
                    DecisionCedar::new(p.into_iter().collect(), Empreinte::depuis_octets(v), r)
                        .unwrap()
                });
            prop_oneof![
                Just(Acteur::Systeme),
                (uuid_(), symbole(), uuid_(), decision).prop_map(
                    |(id, role, session, decision)| {
                        Acteur::Utilisateur {
                            id,
                            role,
                            session,
                            decision,
                        }
                    }
                ),
            ]
        }

        prop_compose! {
            fn entree()(
                portee in portee(),
                rang in 1u64..,
                instant in instant(),
                acteur in acteur(),
                action in symbole(),
                ressource in (symbole(), uuid_()),
                motif in proptest::option::of("[^\\p{Cc}]{0,20}[^\\p{Cc}\\s][^\\p{Cc}]{0,20}"),
                donnees in proptest::collection::btree_map(symbole(), texte(), 0..3),
                champs in proptest::collection::btree_map(
                    symbole(),
                    (proptest::option::of(texte()), proptest::option::of(texte())),
                    0..3,
                ),
            ) -> EntreeAudit {
                let mut e = Evenement::new(
                    acteur,
                    action,
                    Ressource { genre: ressource.0, id: ressource.1 },
                );
                if let Some(m) = motif {
                    e = e.avec_motif(Texte::parse(&m).unwrap()).unwrap();
                }
                for (nom, valeur) in donnees {
                    e = e.avec_donnee(nom, valeur).unwrap();
                }
                for (nom, (avant, apres)) in champs {
                    let apres = if avant.is_none() && apres.is_none() {
                        Some(Texte::parse("").unwrap())
                    } else {
                        apres
                    };
                    e = e.avec_champ(Champ::new(nom, avant, apres).unwrap()).unwrap();
                }
                EntreeAudit::nouvelle(portee, rang, instant, e).unwrap()
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(512))]

            /// Menace F7.3 : la sérialisation est injective. Toute entrée se relit exactement
            /// depuis sa forme canonique ; deux entrées distinctes ont donc des formes
            /// canoniques distinctes.
            #[test]
            fn serialisation_injective(a in entree(), b in entree()) {
                let (ta, tb) = (a.serialiser(), b.serialiser());
                prop_assert_eq!(EntreeAudit::lire(&ta), Ok(a.clone()));
                prop_assert_eq!(a == b, ta == tb);
            }
        }
    }
}
