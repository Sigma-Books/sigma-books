// Ré-authentification avant une action sensible (ADR 0012, F1.6 : changements de droits,
// réinitialisation, enrôlement et suppression d'une méthode de connexion).
//
// L'interface ne décide de rien : elle lit `fraiche` dans l'état de la session rendu par le
// serveur, et propose de se ré-authentifier si la session ne l'est plus. Le serveur refuse de toute
// façon une action sensible sur une session non fraîche (403).

import { api } from './application.ts';
import { executerSensible as executer, type IssueSensible } from './fraicheur.ts';

class EtatReauthentification {
	ouverte = $state(false);
	private attente: Promise<boolean> | null = null;
	private resoudre: ((reussie: boolean) => void) | null = null;

	/** Ouvre la boîte de ré-authentification ; vrai si elle a réussi, faux si elle est annulée. */
	demander(): Promise<boolean> {
		this.attente ??= new Promise<boolean>((resolve) => {
			this.resoudre = resolve;
			this.ouverte = true;
		});
		return this.attente;
	}

	terminer(reussie: boolean): void {
		this.ouverte = false;
		const resoudre = this.resoudre;
		this.attente = null;
		this.resoudre = null;
		resoudre?.(reussie);
	}
}

export const reauthentification = new EtatReauthentification();

/**
 * Vrai si la session est fraîche, ou si l'utilisateur vient de se ré-authentifier. Une erreur de
 * l'API (session expirée, serveur injoignable) remonte à l'appelant.
 */
export async function assurerFraicheur(): Promise<boolean> {
	const session = await api.ouvrirSession();
	if (session.fraiche) return true;
	return reauthentification.demander();
}

/**
 * Exécute une action sensible : ré-authentification d'abord si la session n'est plus fraîche, et
 * un seul rejeu si elle expire entre la vérification et l'action (voir `fraicheur.ts`).
 */
export function executerSensible<T>(travail: () => Promise<T>): Promise<IssueSensible<T>> {
	return executer(travail, {
		assurer: assurerFraicheur,
		estFraiche: async () => (await api.ouvrirSession()).fraiche
	});
}
