<script lang="ts">
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { contesterReinitialisation } from '#lib/api/ressources.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let enCours = $state(false);
	let fait = $state(false);
	let erreur = $state<ErreurApi | null>(null);

	async function contester(): Promise<void> {
		if (data.contenu === null) return;
		enCours = true;
		erreur = null;
		try {
			await contesterReinitialisation(api, data.cabinet, data.reinitialisation);
			fait = true;
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = false;
		}
	}
</script>

<svelte:head>
	<title>Contester une réinitialisation — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else}
	<h1>Contester une réinitialisation de mon accès</h1>
	{#if !data.contenu.ok}
		<MessageErreur erreur={data.contenu.erreur} contexte="Le cabinet n’a pas pu être ouvert." />
	{:else if fait}
		<div class="message message-succes" role="status">
			<p>
				Réinitialisation contestée : elle est annulée et les administrateurs du cabinet sont
				prévenus. Votre accès actuel est conservé.
			</p>
		</div>
		<p><a href={resolve('/(app)')}>Retour à l’accueil</a></p>
	{:else}
		<p>
			Un administrateur a demandé la réinitialisation de votre accès (retrait de vos moyens de
			connexion et de vos sessions). Si vous n’êtes pas à l’origine de cette demande ou si elle vous
			surprend, contestez-la : elle sera annulée.
		</p>
		{#if erreur}
			<MessageErreur {erreur} contexte="La contestation n’a pas abouti." />
		{/if}
		<button type="button" class="bouton" onclick={contester} disabled={enCours}>
			Contester cette réinitialisation
		</button>
	{/if}
{/if}
