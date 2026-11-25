<script lang="ts">
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import ListeBrouillard from '#lib/composants/brouillard/ListeBrouillard.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>Brouillard — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Brouillard</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Le brouillard n’a pas pu être chargé." />
{:else}
	{@const { contexte, valeur } = data.contenu.valeur}
	<h1>Brouillard — {contexte.nomDossier.nom}</h1>
	<NavigationDossier cabinet={contexte.cabinet} dossier={contexte.dossier} actif="brouillard" />
	{#key data.contenu}
		<ListeBrouillard
			cabinet={contexte.cabinet}
			dossier={contexte.dossier}
			role={contexte.membre.role}
			journaux={valeur.journaux}
			exercices={valeur.exercices}
			initiale={valeur.page}
			filtre={data.filtre}
		/>
	{/key}
{/if}
