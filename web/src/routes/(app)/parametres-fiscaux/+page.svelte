<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import ParametresApplicables from '#lib/composants/fiscal/ParametresApplicables.svelte';
	import TableVersions from '#lib/composants/fiscal/TableVersions.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import { afficherDate, estDateIso } from '#lib/dates.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	// La date du champ part de celle de la version affichée (relue à chaque chargement).
	let dateSaisie = $derived(data.contenu?.ok ? data.contenu.valeur.valeur.applicable.date : '');

	async function appliquer(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		if (!data.contenu?.ok || !estDateIso(dateSaisie)) return;
		const { cabinet, dossier } = data.contenu.valeur.contexte;
		await goto(
			resolve(`parametres-fiscaux/?cabinet=${cabinet}&dossier=${dossier}&date=${dateSaisie}`)
		);
	}
</script>

<svelte:head>
	<title>Paramètres fiscaux — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Paramètres fiscaux</h1>
	<MessageErreur
		erreur={data.contenu.erreur}
		contexte="Les paramètres fiscaux n’ont pas pu être chargés."
	/>
{:else}
	{@const { contexte, valeur } = data.contenu.valeur}
	{@const { liste, applicable } = valeur}
	<h1>Paramètres fiscaux — {contexte.nomDossier.nom}</h1>
	<NavigationDossier
		cabinet={contexte.cabinet}
		dossier={contexte.dossier}
		actif="parametres"
		role={contexte.membre.role}
	/>

	<section aria-labelledby="titre-versions">
		<h2 id="titre-versions">Versions</h2>
		<p class="aide">
			Pays du dossier : {liste.pays}. Les versions du pays viennent de son paquet de paramètres ;
			une version propre au dossier l’emporte sur celles du pays aux dates où elle s’applique. {traduire(
				'parametres.immuable'
			)}
		</p>
		{#if liste.versions.length === 0}
			<p>Le pays du dossier n’a aucun paquet de paramètres fiscaux.</p>
		{:else}
			<TableVersions versions={liste.versions} regional={contexte.regional} />
		{/if}
	</section>

	<section aria-labelledby="titre-applicable">
		<h2 id="titre-applicable">Version applicable à une date</h2>
		<form class="barre-actions" onsubmit={appliquer}>
			<div class="champ">
				<label for="date-applicable">Date</label>
				<input id="date-applicable" type="date" required bind:value={dateSaisie} />
			</div>
			<button class="bouton bouton-secondaire" type="submit">Afficher</button>
		</form>
		{#if applicable.version === null}
			<p>
				Aucune version de paramètres fiscaux ne s’applique au {afficherDate(
					applicable.date,
					contexte.regional
				)} : aucune étiquette fiscale n’est possible à cette date.
			</p>
		{:else}
			<p>
				Au {afficherDate(applicable.date, contexte.regional)} : version
				<strong class="mono">{applicable.version.identifiant}</strong>
				({applicable.version.portee === 'pays' ? 'paquet du pays' : 'version du dossier'}, depuis le
				{afficherDate(applicable.version.date_effet, contexte.regional)}{applicable.version.date_fin
					? `, jusqu’au ${afficherDate(applicable.version.date_fin, contexte.regional)}`
					: ''}).
				{#if applicable.retention_annees !== null}
					Conservation des pièces : {applicable.retention_annees} ans.
				{/if}
			</p>
			<ParametresApplicables {applicable} />
		{/if}
	</section>
{/if}
