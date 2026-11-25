<!--
	Ré-authentification de la session (mot de passe de secours et code TOTP), exigée avant la
	validation par lot. Le serveur décide ; ce formulaire n'enregistre ni mot de passe ni code :
	les champs sont vidés après l'envoi.
-->
<script lang="ts">
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { reauthentifier } from '#lib/api/reauthentification.ts';
	import { api } from '#lib/application.ts';
	import MessageMetier from './MessageMetier.svelte';

	interface Props {
		/** Appelé quand la session est de nouveau fraîche. */
		surReussite: () => void;
		surAnnulation: () => void;
	}

	let { surReussite, surAnnulation }: Props = $props();

	let motDePasse = $state('');
	let code = $state('');
	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);

	async function envoyer(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		enCours = true;
		erreur = null;
		try {
			await reauthentifier(api, motDePasse, code);
			motDePasse = '';
			code = '';
			surReussite();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
			motDePasse = '';
			code = '';
		} finally {
			enCours = false;
		}
	}
</script>

<form class="reauth" onsubmit={envoyer} aria-labelledby="titre-reauth">
	<h2 id="titre-reauth">Confirmez votre identité</h2>
	<p class="aide">
		La validation par lot exige une authentification récente. Saisissez votre mot de passe et le
		code à usage unique de votre application d’authentification.
	</p>
	{#if erreur}
		<MessageMetier {erreur} contexte="La ré-authentification a échoué." />
	{/if}
	<div class="champ">
		<label for="reauth-mdp">Mot de passe</label>
		<input
			id="reauth-mdp"
			type="password"
			autocomplete="current-password"
			required
			bind:value={motDePasse}
		/>
	</div>
	<div class="champ">
		<label for="reauth-code">Code à 6 chiffres</label>
		<input
			id="reauth-code"
			type="text"
			inputmode="numeric"
			autocomplete="one-time-code"
			pattern="[0-9 ]*"
			required
			bind:value={code}
		/>
	</div>
	<p class="actions">
		<button type="submit" class="bouton" disabled={enCours}>Confirmer</button>
		<button type="button" class="bouton bouton-secondaire" onclick={surAnnulation}>Annuler</button>
	</p>
</form>

<style>
	.reauth {
		border: 1px solid var(--couleur-separateur);
		border-radius: var(--rayon);
		padding: var(--espace-4);
		margin-bottom: var(--espace-4);
		max-width: 28rem;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-2);
	}
</style>
