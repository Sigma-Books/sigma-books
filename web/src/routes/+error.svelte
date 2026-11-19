<!-- Page d'erreur générale : aucun détail technique. -->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
</script>

<svelte:head>
	<title>Erreur — Sigma Books</title>
</svelte:head>

<main id="contenu" class="conteneur">
	<h1>{page.status === 404 ? 'Page introuvable' : 'Une erreur est survenue'}</h1>
	<p><a href={resolve('/(app)')}>Revenir à l’accueil</a></p>
</main>

<style>
	main {
		padding-block: var(--espace-6);
	}
</style>
