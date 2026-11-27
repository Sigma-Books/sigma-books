// Déroulement de la création d'une version de paramètres fiscaux propre au dossier : lecture de la
// version de base, vérification, confirmation, création. Module pur (dépendances injectées), testé
// sous `node --test` ; l'écran n'en est que l'affichage.
//
// Garanties : une lecture périmée (autre date) n'écrit jamais l'état ; la vérification attend la
// lecture de la date saisie ; une seconde demande de création pendant la première est ignorée ; un
// contrôleur détruit (écran quitté, autre dossier) n'écrit plus rien ; une réponse 201 illisible est
// signalée « peut-être créée » (jamais proposée à nouveau) ; la ré-authentification passe par
// `executerSensible` (reprise unique).

import { ErreurApi } from './api/erreurs.ts';
import type {
	Applicable,
	CorpsVersion,
	VersionCreee,
	VersionFiscale
} from './api/parametres-fiscaux.ts';
import { estDateIso } from './dates.ts';
import type { IssueSensible } from './fraicheur.ts';
import {
	corpsVersion,
	diagnostiquerVersion,
	type EtatVersions,
	type FormulaireVersion,
	type ProblemeVersion
} from './parametres-fiscaux.ts';
import type { Regional } from './regional.ts';

export type PhaseCreation = 'saisie' | 'confirmation' | 'fait';

/** Ce que l'écran affiche. Un nouvel objet à chaque changement. */
export interface VueCreation {
	phase: PhaseCreation;
	/** Version applicable à la date d'effet saisie (taux, régimes), une fois lue. */
	base: Applicable | null;
	lectureEnCours: boolean;
	erreurBase: ErreurApi | null;
	problemes: ProblemeVersion[];
	enCours: boolean;
	erreur: ErreurApi | null;
	/** La ré-authentification demandée n'a pas eu lieu. */
	reauthAnnulee: boolean;
	creee: VersionCreee | null;
	/** Réponse 201 illisible : la version existe peut-être. */
	incertaine: boolean;
}

export interface DependancesCreation {
	lireBase(date: string): Promise<Applicable>;
	creer(corps: CorpsVersion): Promise<VersionCreee>;
	executerSensible<T>(travail: () => Promise<T>): Promise<IssueSensible<T>>;
	/** Recharge la liste des versions (ne doit pas faire échouer la création si elle échoue). */
	actualiser(): Promise<void>;
	/** Saisies actuelles du formulaire. */
	formulaire(): FormulaireVersion;
	/** Repart de zéro sur les taux et retraits (la base a changé) ou sur tout le formulaire. */
	reinitialiserSaisies(tout: boolean): void;
	aujourdhui(): string;
	versions(): readonly VersionFiscale[];
	regional(): Pick<Regional, 'langue'> & { pays?: string };
}

/** Vrai si la création a pu aboutir : 201 reçu mais réponse illisible. */
export function versionPossiblementCreee(erreur: ErreurApi): boolean {
	return erreur.code === 'reponse_invalide' && erreur.statut === 201;
}

export class CreationVersion {
	vue: VueCreation = {
		phase: 'saisie',
		base: null,
		lectureEnCours: false,
		erreurBase: null,
		problemes: [],
		enCours: false,
		erreur: null,
		reauthAnnulee: false,
		creee: null,
		incertaine: false
	};

	private lecture = 0;
	private dateLue: string | null = null;
	private dateEnLecture: string | null = null;
	private enAttente: Promise<void> | null = null;
	private actif = true;

	private readonly d: DependancesCreation;
	private readonly surChangement: () => void;

	constructor(d: DependancesCreation, surChangement: () => void) {
		this.d = d;
		this.surChangement = surChangement;
	}

	private maj(partiel: Partial<VueCreation>): void {
		if (!this.actif) return;
		this.vue = { ...this.vue, ...partiel };
		this.surChangement();
	}

	/** L'écran est quitté : plus aucune écriture d'état, plus aucune lecture ne compte. */
	detruire(): void {
		this.actif = false;
		this.lecture += 1;
	}

	private etat(): EtatVersions {
		return {
			aujourdhui: this.d.aujourdhui(),
			versions: this.d.versions(),
			base: this.vue.base,
			regional: this.d.regional()
		};
	}

	/**
	 * La date d'effet a changé : lit la version applicable (rôles de taux, régimes offerts). Une
	 * lecture plus récente rend les précédentes sans effet.
	 */
	choisirDate(date: string): Promise<void> {
		const numero = ++this.lecture;
		this.dateLue = null;
		this.dateEnLecture = null;
		this.enAttente = null;
		this.maj({ base: null, erreurBase: null, lectureEnCours: false });
		if (!estDateIso(date)) return Promise.resolve();
		this.dateEnLecture = date;
		this.maj({ lectureEnCours: true });
		const travail = (async () => {
			try {
				const lue = await this.d.lireBase(date);
				if (numero !== this.lecture) return;
				this.dateLue = date;
				// Les taux et retraits saisis portaient sur la base précédente.
				this.d.reinitialiserSaisies(false);
				this.maj({ base: lue, lectureEnCours: false });
			} catch (e) {
				if (numero !== this.lecture) return;
				if (e instanceof ErreurApi) this.maj({ erreurBase: e, lectureEnCours: false });
				else if (e instanceof TypeError) this.maj({ lectureEnCours: false });
				else throw e;
			}
		})();
		this.enAttente = travail;
		return travail;
	}

	/**
	 * Vérifie le formulaire : attend la lecture de la date saisie (ou la lance), puis contrôle. Sans
	 * problème, passe à la confirmation.
	 */
	async verifier(): Promise<void> {
		const date = this.d.formulaire().dateEffet;
		if (estDateIso(date) && this.dateLue !== date) {
			if (this.dateEnLecture === date && this.enAttente !== null) await this.enAttente;
			else await this.choisirDate(date);
		}
		if (!this.actif) return;
		// Si la lecture a échoué, c'est son erreur qui s'affiche, pas « aucune version applicable ».
		if (this.vue.erreurBase !== null) {
			this.maj({ problemes: [], erreur: null, reauthAnnulee: false });
			return;
		}
		const problemes = diagnostiquerVersion(this.d.formulaire(), this.etat());
		this.maj({
			problemes,
			erreur: null,
			reauthAnnulee: false,
			phase: problemes.length === 0 ? 'confirmation' : 'saisie'
		});
	}

	modifier(): void {
		this.maj({ phase: 'saisie' });
	}

	/** Crée la version. Une demande pendant une création en cours est ignorée. */
	async creer(): Promise<void> {
		if (this.vue.enCours || !this.actif) return;
		const corps = corpsVersion(this.d.formulaire(), this.etat());
		if (corps === null) {
			this.maj({ phase: 'saisie' });
			return;
		}
		this.maj({ enCours: true, erreur: null, reauthAnnulee: false });
		try {
			const issue = await this.d.executerSensible(() => this.d.creer(corps));
			if (!this.actif) return;
			if (!issue.fait) {
				this.maj({ reauthAnnulee: true, phase: 'saisie' });
				return;
			}
			this.termine({ creee: issue.valeur, incertaine: false });
			await this.actualiser();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			if (versionPossiblementCreee(e)) {
				this.termine({ creee: null, incertaine: true });
				await this.actualiser();
			} else {
				this.maj({ erreur: e, phase: 'saisie' });
			}
		} finally {
			this.maj({ enCours: false });
		}
	}

	private termine(p: Pick<VueCreation, 'creee' | 'incertaine'>): void {
		this.lecture += 1;
		this.dateLue = null;
		this.d.reinitialiserSaisies(true);
		this.maj({ ...p, phase: 'fait', base: null, problemes: [] });
	}

	private async actualiser(): Promise<void> {
		try {
			await this.d.actualiser();
		} catch {
			// La liste se rechargera à la prochaine visite ; la création, elle, est faite.
		}
	}

	/** Une autre version : retour à la saisie. */
	recommencer(): void {
		this.maj({ creee: null, incertaine: false, phase: 'saisie', reauthAnnulee: false });
	}
}
