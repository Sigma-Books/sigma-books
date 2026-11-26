<script lang="ts">
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { lireAlertes, type Alertes } from '#lib/api/editions.ts';
	import { lireEtatIntegrite, type EtatIntegrite } from '#lib/api/integrite.ts';
	import { messageEdition } from '#lib/api/messages-editions.ts';
	import type { Resultat } from '#lib/api/resultat.ts';
	import type { Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { integriteIndisponible, peutDireAucuneAlerte } from '#lib/alertes.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { messageAlerteBrouillard, messageAlerteEcheance } from '#lib/editions-affichage.ts';
	import { dateHeure } from '#lib/instants.ts';
	import { DELAI_RETARD_HORODATAGE_HEURES, PREAVIS_ECHEANCE_JOURS } from '#lib/seuils.ts';
	import '#lib/styles/editions.css';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let alertes = $state<Resultat<Alertes> | null>(null);
	let integrite = $state<Resultat<EtatIntegrite> | null>(null);
	let enCours = $state(false);
	/** Change avec le dossier de l'URL : une réponse d'un autre dossier est ignorée. */
	let generation = 0;
	const cleDossier = $derived(
		data.contenu?.ok
			? `${data.contenu.valeur.contexte.cabinet}/${data.contenu.valeur.contexte.dossier}`
			: ''
	);
	$effect.pre(() => {
		void cleDossier;
		generation += 1;
		enCours = false;
	});

	$effect(() => {
		if (data.contenu?.ok) {
			alertes = data.contenu.valeur.valeur.alertes;
			integrite = data.contenu.valeur.valeur.integrite;
		}
	});

	async function tenter<T>(travail: () => Promise<T>): Promise<Resultat<T>> {
		try {
			return { ok: true, valeur: await travail() };
		} catch (erreur) {
			if (erreur instanceof ErreurApi) return { ok: false, erreur };
			throw erreur;
		}
	}

	async function actualiser(cabinet: Uuid, dossier: Uuid): Promise<void> {
		const g = generation;
		enCours = true;
		const [a, i] = await Promise.all([
			tenter(() => lireAlertes(api, cabinet, dossier)),
			tenter(() => lireEtatIntegrite(api, cabinet, dossier))
		]);
		if (g !== generation) return;
		[alertes, integrite] = [a, i];
		enCours = false;
	}

	const retardHorodatage = $derived(integrite?.ok === true && integrite.valeur.horodatage.retard);
	const total = $derived(
		(alertes?.ok === true
			? alertes.valeur.brouillard_non_valide.length + alertes.valeur.echeances_de_cloture.length
			: 0) + (retardHorodatage ? 1 : 0)
	);
</script>

<svelte:head>
	<title>Alertes — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Alertes</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Les alertes n’ont pas pu être chargées." />
{:else}
	{@const { contexte } = data.contenu.valeur}
	<h1>Alertes — {contexte.nomDossier.nom}</h1>
	<NavigationDossier
		cabinet={contexte.cabinet}
		dossier={contexte.dossier}
		actif="alertes"
		role={contexte.membre.role}
	/>

	<p>
		<button
			class="bouton bouton-secondaire"
			type="button"
			disabled={enCours}
			onclick={() => void actualiser(contexte.cabinet, contexte.dossier)}
		>
			{enCours ? 'Actualisation…' : 'Actualiser'}
		</button>
	</p>

	<div aria-live="polite">
		{#if integriteIndisponible(integrite)}
			<div class="message message-avertissement" role="alert">
				<p>
					État d’intégrité indisponible : le retard d’horodatage n’a pas pu être vérifié. Actualisez
					l’écran ou réessayez plus tard.
				</p>
			</div>
		{/if}
		{#if alertes?.ok === true}
			{#if peutDireAucuneAlerte(total, alertes, integrite)}
				<div class="message message-succes" role="status">
					<p>Aucune alerte au {afficherDate(alertes.valeur.date_du_jour, contexte.regional)}.</p>
				</div>
			{:else if total > 0}
				<p role="status">
					{total} alerte(s) au {afficherDate(alertes.valeur.date_du_jour, contexte.regional)}.
				</p>
			{/if}
		{/if}
	</div>

	{#if retardHorodatage && integrite?.ok === true}
		<section aria-labelledby="titre-horodatage">
			<h2 id="titre-horodatage">Retard d’horodatage</h2>
			<div class="message message-erreur" role="alert">
				<p>
					Des écritures attendent un jeton d’horodatage depuis plus de {DELAI_RETARD_HORODATAGE_HEURES}
					h
					{#if integrite.valeur.horodatage.en_attente_depuis !== null}
						(depuis le {dateHeure(
							integrite.valeur.horodatage.en_attente_depuis,
							contexte.regional
						)}){/if}.
					<a href={resolve(`integrite/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}`)}>
						Voir l’écran Intégrité
					</a>
				</p>
			</div>
		</section>
	{/if}

	<section aria-labelledby="titre-brouillard">
		<h2 id="titre-brouillard">Brouillard non validé en retard</h2>
		{#if alertes === null}
			<p role="status">Chargement…</p>
		{:else if !alertes.ok}
			<div class="message message-erreur" role="alert">
				<p>{messageEdition(alertes.erreur)}</p>
			</div>
		{:else if alertes.valeur.brouillard_non_valide.length === 0}
			<p>Aucun brouillard en retard : toutes les périodes terminées sont validées.</p>
		{:else}
			<ul class="alertes">
				{#each alertes.valeur.brouillard_non_valide as a, i (i)}
					<li class="message message-avertissement">
						{messageAlerteBrouillard(a, contexte.regional)}
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<section aria-labelledby="titre-echeances">
		<h2 id="titre-echeances">Échéances de clôture ({PREAVIS_ECHEANCE_JOURS} jours ou dépassées)</h2>
		{#if alertes === null}
			<p role="status">Chargement…</p>
		{:else if !alertes.ok}
			<div class="message message-erreur" role="alert">
				<p>{messageEdition(alertes.erreur)}</p>
			</div>
		{:else if alertes.valeur.echeances_de_cloture.length === 0}
			<p>Aucune échéance de clôture proche ou dépassée.</p>
		{:else}
			<ul class="alertes">
				{#each alertes.valeur.echeances_de_cloture as a, i (i)}
					<li class={a.depassee ? 'message message-erreur' : 'message message-avertissement'}>
						{messageAlerteEcheance(a, contexte.regional)}
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<p class="aide">
		Les exports d’intégrité et d’audit, le retard d’horodatage et les changements de droits sont
		aussi envoyés par courriel aux administrateurs du cabinet.
	</p>
{/if}

<style>
	.alertes {
		list-style: none;
		margin: 0 0 var(--espace-5);
		padding: 0;
	}
</style>
