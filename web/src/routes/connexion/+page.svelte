<!--
	Connexion : passkey (méthode principale), ou secours (mot de passe et code TOTP), ou code de
	récupération. Tout échec donne le même message, quelle qu'en soit la cause : le serveur ne
	distingue pas un courriel inconnu d'un facteur faux (anti-énumération), l'interface non plus.
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		connecterParCodeRecuperation,
		connecterParPasskey,
		connecterParSecours,
		defiConnexionPasskey
	} from '#lib/api/ressources.ts';
	import { api, sessionOuverte } from '#lib/application.ts';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { authentifier, ErreurPasskey, passkeysPrisesEnCharge } from '#lib/passkeys.ts';

	type Mode = 'passkey' | 'secours' | 'code';

	const prisEnCharge = $derived(passkeysPrisesEnCharge());
	let choix = $state<Mode | null>(null);
	const mode = $derived<Mode>(choix ?? (prisEnCharge ? 'passkey' : 'secours'));
	let courriel = $state('');
	let motDePasse = $state('');
	let code = $state('');
	let enCours = $state(false);
	let message = $state('');
	let erreurTechnique = $state<ErreurApi | null>(null);
	const expiree = $derived(page.url.searchParams.get('session') === 'expiree');

	/** Erreurs d'identification : une seule phrase pour toutes. */
	const ECHEC = 'Connexion impossible. Vérifiez vos informations et réessayez.';

	async function terminer(nature: string): Promise<void> {
		sessionOuverte();
		motDePasse = '';
		code = '';
		await api.ouvrirSession();
		await goto(nature === 'complete' ? resolve('/(app)') : resolve('compte/'));
	}

	async function executer(travail: () => Promise<string>): Promise<void> {
		enCours = true;
		message = '';
		erreurTechnique = null;
		try {
			await terminer(await travail());
		} catch (e) {
			motDePasse = '';
			code = '';
			if (e instanceof ErreurPasskey) {
				message =
					e.raison === 'annulee'
						? 'La vérification a été annulée ou a expiré. Réessayez.'
						: 'Ce navigateur ne prend pas en charge les passkeys. Utilisez la méthode de secours.';
			} else if (e instanceof ErreurApi) {
				if (e.code === 'non_authentifie') message = ECHEC;
				else erreurTechnique = e;
			} else {
				throw e;
			}
		} finally {
			enCours = false;
		}
	}

	function parPasskey(evenement: SubmitEvent): void {
		evenement.preventDefault();
		void executer(async () => {
			const { defi } = await defiConnexionPasskey(api, courriel);
			const credential = await authentifier(defi);
			return (await connecterParPasskey(api, courriel, credential)).nature;
		});
	}

	function parSecours(evenement: SubmitEvent): void {
		evenement.preventDefault();
		void executer(async () => (await connecterParSecours(api, courriel, motDePasse, code)).nature);
	}

	function parCode(evenement: SubmitEvent): void {
		evenement.preventDefault();
		void executer(async () => (await connecterParCodeRecuperation(api, courriel, code)).nature);
	}
</script>

<svelte:head>
	<title>Connexion — Sigma Books</title>
</svelte:head>

<main id="contenu" class="conteneur etroit">
	<h1>Connexion</h1>

	{#if expiree}
		<div class="message message-avertissement" role="status">
			<p>Votre session a expiré ou a été fermée. Reconnectez-vous.</p>
		</div>
	{/if}

	<div role="group" aria-label="Méthode de connexion" class="methodes">
		{#if prisEnCharge}
			<button
				type="button"
				class="bouton bouton-secondaire"
				aria-pressed={mode === 'passkey'}
				onclick={() => (choix = 'passkey')}
			>
				Passkey
			</button>
		{/if}
		<button
			type="button"
			class="bouton bouton-secondaire"
			aria-pressed={mode === 'secours'}
			onclick={() => (choix = 'secours')}
		>
			Mot de passe et code
		</button>
		<button
			type="button"
			class="bouton bouton-secondaire"
			aria-pressed={mode === 'code'}
			onclick={() => (choix = 'code')}
		>
			Code de récupération
		</button>
	</div>

	<div role="status" aria-live="polite">
		{#if message}
			<div class="message message-erreur"><p>{message}</p></div>
		{/if}
	</div>
	{#if erreurTechnique}
		<MessageErreur erreur={erreurTechnique} contexte="La connexion n’a pas pu aboutir." />
	{/if}

	{#if mode === 'passkey'}
		<form onsubmit={parPasskey}>
			<div class="champ">
				<label for="courriel">Adresse de courriel</label>
				<input
					id="courriel"
					name="courriel"
					type="email"
					autocomplete="username webauthn"
					required
					maxlength="254"
					bind:value={courriel}
				/>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>
				{enCours ? 'Vérification…' : 'Se connecter avec ma passkey'}
			</button>
		</form>
	{:else if mode === 'secours'}
		<form onsubmit={parSecours}>
			<div class="champ">
				<label for="courriel">Adresse de courriel</label>
				<input
					id="courriel"
					name="courriel"
					type="email"
					autocomplete="username"
					required
					maxlength="254"
					bind:value={courriel}
				/>
			</div>
			<div class="champ">
				<label for="mdp">Mot de passe</label>
				<input
					id="mdp"
					name="mot_de_passe"
					type="password"
					autocomplete="current-password"
					required
					maxlength="1024"
					bind:value={motDePasse}
				/>
			</div>
			<div class="champ">
				<label for="code">Code de l’application d’authentification</label>
				<input
					id="code"
					name="code"
					type="text"
					inputmode="numeric"
					autocomplete="one-time-code"
					pattern="[0-9 ]*"
					required
					maxlength="12"
					bind:value={code}
				/>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>
				{enCours ? 'Vérification…' : 'Se connecter'}
			</button>
		</form>
	{:else}
		<form onsubmit={parCode}>
			<p class="aide">
				Un code de récupération ouvre une session limitée à l’enrôlement d’une nouvelle méthode de
				connexion. Chaque code ne sert qu’une fois.
			</p>
			<div class="champ">
				<label for="courriel">Adresse de courriel</label>
				<input
					id="courriel"
					name="courriel"
					type="email"
					autocomplete="username"
					required
					maxlength="254"
					bind:value={courriel}
				/>
			</div>
			<div class="champ">
				<label for="code-recup">Code de récupération</label>
				<input
					id="code-recup"
					name="code"
					type="text"
					autocomplete="off"
					autocapitalize="characters"
					spellcheck="false"
					required
					maxlength="64"
					bind:value={code}
				/>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>
				{enCours ? 'Vérification…' : 'Utiliser le code'}
			</button>
		</form>
	{/if}
</main>

<style>
	main {
		padding-block: var(--espace-6);
	}

	.etroit {
		max-width: 32rem;
	}

	.methodes {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-2);
		margin-bottom: var(--espace-5);
	}

	.methodes button[aria-pressed='true'] {
		background: var(--couleur-surface);
		border-width: 2px;
		font-weight: 700;
	}
</style>
