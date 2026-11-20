<script lang="ts">
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>Administration — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Administration</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Le cabinet n’a pas pu être chargé." />
{:else}
	<h1>Administration de {data.contenu.valeur.nom}</h1>
	<div class="message message-avertissement">
		<p>
			La gestion des utilisateurs, des invitations, des rôles et des accès aux dossiers sera
			disponible prochainement.
		</p>
	</div>
{/if}
