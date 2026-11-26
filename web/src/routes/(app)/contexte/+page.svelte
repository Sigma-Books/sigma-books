<script lang="ts">
	import { goto } from '$app/navigation';
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import type { PackPays } from '#lib/api/pays.ts';
	import { modifierContexte } from '#lib/api/ressources.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import FormulaireContexte from '#lib/composants/FormulaireContexte.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import MessageMetier from '#lib/composants/MessageMetier.svelte';
	import { ROLES_CONTEXTE, type ContexteDossier } from '#lib/contexte-dossier.ts';
	import {
		formulaireDuDossier,
		preparerModification,
		type FormulaireContexte as Formulaire,
		type Probleme
	} from '#lib/contexte-saisie.ts';
	import { executerSensible } from '#lib/reauthentification.svelte.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let formulaire = $state<Formulaire | null>(null);
	let problemes = $state<Probleme[]>([]);
	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let info = $state<string | null>(null);
	let formulaireDom = $state<HTMLFormElement>();
	let zoneErreur = $state<HTMLElement>();

	/** Dossier affiché maintenant (une requête lancée pour un autre dossier est périmée). */
	const dossierCourant = () => (data.contenu?.ok ? data.contenu.valeur.contexte.dossier : null);

	/** Amène le focus sur le message d'erreur, ou sur le premier champ refusé : le bouton est en bas. */
	async function montrerErreur(): Promise<void> {
		await tick();
		const cible =
			erreur !== null || info !== null
				? zoneErreur
				: formulaireDom?.querySelector<HTMLElement>('[aria-invalid="true"]');
		cible?.focus();
		cible?.scrollIntoView({ block: 'center' });
	}

	// Le formulaire part du contexte lu (relu à chaque chargement d'un autre dossier).
	$effect.pre(() => {
		const lu = data.contenu?.ok ? data.contenu.valeur : null;
		const pack = lu?.valeur?.ok ? lu.valeur.valeur : null;
		formulaire =
			lu !== null && pack !== null ? formulaireDuDossier(lu.contexte.nomDossier, pack) : null;
		problemes = [];
		erreur = null;
		info = null;
		enCours = false;
	});

	/**
	 * Envoie les champs changés. Pays et devise exigent une session fraîche : la boîte de
	 * ré-authentification s'ouvre au besoin et la requête est rejouée une seule fois
	 * (`executerSensible`). Les refus du serveur (409, 422) sont affichés tels quels.
	 */
	async function enregistrer(evenement: SubmitEvent, contexte: ContexteDossier, pack: PackPays) {
		evenement.preventDefault();
		if (formulaire === null) return;
		erreur = null;
		info = null;
		const initial = formulaireDuDossier(contexte.nomDossier, pack);
		const modification = preparerModification(initial, formulaire, pack);
		if (modification.etat === 'invalide') {
			problemes = modification.problemes;
			await montrerErreur();
			return;
		}
		problemes = [];
		if (modification.etat === 'inchange') {
			info = 'Aucun changement à enregistrer.';
			await montrerErreur();
			return;
		}
		enCours = true;
		try {
			const envoyer = () =>
				modifierContexte(api, contexte.cabinet, contexte.dossier, modification.corps);
			if (modification.sensible) {
				const issue = await executerSensible(envoyer);
				if (dossierCourant() !== contexte.dossier) return;
				if (!issue.fait) {
					info = 'Modification annulée : la ré-authentification n’a pas eu lieu.';
					await montrerErreur();
					return;
				}
			} else {
				await envoyer();
			}
			// L'utilisateur a pu ouvrir un autre dossier pendant l'attente : ne pas l'en ramener.
			if (dossierCourant() !== contexte.dossier) return;
			await goto(resolve(`dossier/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}`));
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			if (dossierCourant() !== contexte.dossier) return;
			erreur = e;
			await montrerErreur();
		} finally {
			if (dossierCourant() === contexte.dossier) enCours = false;
		}
	}
</script>

<svelte:head>
	<title>Contexte du dossier — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Contexte du dossier</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Le dossier n’a pas pu être chargé." />
{:else}
	{@const { contexte, valeur: pack } = data.contenu.valeur}
	<h1>Contexte du dossier — {contexte.nomDossier.nom}</h1>
	<p>
		<a href={resolve(`dossier/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}`)}>
			Retour au dossier
		</a>
	</p>

	{#if !ROLES_CONTEXTE.includes(contexte.membre.role) || pack === null}
		<div class="message message-avertissement" role="status">
			<p>Seul un administrateur du cabinet peut modifier le contexte d’un dossier.</p>
		</div>
	{:else if !pack.ok}
		<MessageErreur
			erreur={pack.erreur}
			contexte="La liste des pays n’a pas pu être chargée : la modification est impossible."
		/>
	{:else if formulaire !== null}
		<p class="aide">
			Le fuseau, la langue et la forme juridique se modifient à tout moment. Le pays, la devise de
			tenue (donc son échelle) et les identifiants légaux ne se modifient que tant que le dossier
			n’a aucune écriture validée ; le pays et la devise exigent en outre qu’aucun brouillon ne soit
			en cours et une nouvelle authentification. Après la première écriture validée, ils sont figés.
		</p>
		<div bind:this={zoneErreur} tabindex="-1">
			{#if erreur}
				<MessageMetier {erreur} contexte="Le contexte n’a pas été modifié." />
			{/if}
			{#if info}
				<div class="message message-avertissement" role="status"><p>{info}</p></div>
			{/if}
		</div>
		<form
			bind:this={formulaireDom}
			onsubmit={(e) => enregistrer(e, contexte, pack.valeur)}
			novalidate
		>
			<FormulaireContexte
				pack={pack.valeur}
				bind:formulaire
				{problemes}
				paysInitial={contexte.nomDossier.pays}
				modification
			/>
			<button type="submit" class="bouton" disabled={enCours}>Enregistrer</button>
		</form>
	{/if}
{/if}
