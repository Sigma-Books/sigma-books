<script lang="ts">
	import { resolve } from '$app/paths';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { libelleRole } from '#lib/libelles.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>Cabinets — Sigma Books</title>
</svelte:head>

<h1>Choisir un cabinet</h1>

{#if !data.cabinets.ok}
	<MessageErreur erreur={data.cabinets.erreur} contexte="Les cabinets n’ont pas pu être chargés." />
{:else if data.cabinets.valeur.length === 0}
	<p>Vous n’êtes membre d’aucun cabinet actif.</p>
{:else}
	<ul class="liste">
		{#each data.cabinets.valeur as cabinet (cabinet.id)}
			<li>
				<a class="liste-lien" href={resolve(`dossiers/?cabinet=${cabinet.id}`)}>
					<span class="nom">{cabinet.nom}</span>
					<span class="detail">{libelleRole(cabinet.role)}</span>
				</a>
			</li>
		{/each}
	</ul>
{/if}
