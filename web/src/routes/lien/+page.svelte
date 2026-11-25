<!--
	Lien d'invitation ou de réinitialisation reçu par courriel. Rien n'est consommé à l'ouverture de
	la page : le lien, à usage unique, n'est utilisé qu'au clic (un aperçu ou un antivirus qui
	chargerait la page ne le brûle pas). Il ouvre une session limitée à l'enrôlement d'une méthode
	de connexion.
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { accepterInvitation, utiliserLien } from '#lib/api/ressources.ts';
	import { api, sessionOuverte } from '#lib/application.ts';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { jetonDuFragment } from '#lib/lien.ts';

	let jeton = $state<string | null>(null);
	let pret = $state(false);
	let sessionComplete = $state(false);
	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let accepte = $state(false);

	$effect(() => {
		jeton = jetonDuFragment(window.location.hash);
		api.ouvrirSession(true).then(
			(session) => {
				sessionComplete = session.nature === 'complete';
				pret = true;
			},
			() => {
				sessionComplete = false;
				pret = true;
			}
		);
	});

	function effacerFragment(): void {
		// Le jeton est consommé : il ne reste pas dans l'adresse ni dans l'historique.
		history.replaceState(null, '', window.location.pathname);
	}

	async function continuer(): Promise<void> {
		if (jeton === null) return;
		enCours = true;
		erreur = null;
		try {
			await utiliserLien(api, jeton);
			effacerFragment();
			jeton = null;
			sessionOuverte();
			await api.ouvrirSession();
			await goto(resolve('compte/'));
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = false;
		}
	}

	async function accepter(): Promise<void> {
		if (jeton === null) return;
		enCours = true;
		erreur = null;
		try {
			await accepterInvitation(api, jeton);
			effacerFragment();
			jeton = null;
			accepte = true;
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = false;
		}
	}
</script>

<svelte:head>
	<title>Lien d’accès — Sigma Books</title>
</svelte:head>

<main id="contenu" class="conteneur etroit">
	<h1>Lien d’accès</h1>

	{#if !pret}
		<p role="status">Chargement…</p>
	{:else if accepte}
		<div class="message message-succes" role="status">
			<p>Invitation acceptée : le cabinet apparaît dans votre liste.</p>
		</div>
		<p><a href={resolve('/(app)')}>Choisir un cabinet</a></p>
	{:else if jeton === null}
		<div class="message message-erreur" role="alert">
			<p>Ce lien est incomplet. Rouvrez le lien tel que vous l’avez reçu par courriel.</p>
		</div>
	{:else}
		{#if erreur}
			<MessageErreur {erreur} contexte="Le lien n’a pas pu être utilisé." />
			{#if erreur.motif === 'compte_existant'}
				<p><a href={resolve('connexion/')}>Se connecter</a>, puis rouvrez ce lien.</p>
			{/if}
		{/if}
		<p>
			Ce lien permet d’enregistrer votre méthode de connexion (passkey de préférence). Il ne
			fonctionne qu’une fois et pendant une durée limitée.
		</p>
		<p>
			<button type="button" class="bouton" onclick={continuer} disabled={enCours}>Continuer</button>
		</p>
		{#if sessionComplete}
			<p class="aide">
				Vous êtes déjà connecté. S’il s’agit d’une invitation à un autre cabinet, vous pouvez
				l’accepter avec votre compte actuel.
			</p>
			<p>
				<button
					type="button"
					class="bouton bouton-secondaire"
					onclick={accepter}
					disabled={enCours}
				>
					Accepter l’invitation avec mon compte
				</button>
			</p>
		{/if}
	{/if}
</main>

<style>
	main {
		padding-block: var(--espace-6);
	}

	.etroit {
		max-width: 32rem;
	}
</style>
