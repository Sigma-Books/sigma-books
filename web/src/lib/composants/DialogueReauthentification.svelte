<!--
	Boîte de ré-authentification (`<dialog>` modale : le focus y reste et Échap l'annule). Deux
	méthodes, selon ce que le serveur dit que le compte possède : passkey, ou mot de passe et code
	TOTP du secours. Aucune valeur saisie n'est conservée après l'envoi.
-->
<script lang="ts">
	import { invalidate } from '$app/navigation';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		defiReauthentificationPasskey,
		lireFacteurs,
		reauthentifierParPasskey,
		reauthentifierParSecours
	} from '#lib/api/ressources.ts';
	import type { FacteursMoi } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { authentifier, ErreurPasskey, passkeysPrisesEnCharge } from '#lib/passkeys.ts';
	import { reauthentification } from '#lib/reauthentification.svelte.ts';

	let dialogue = $state<HTMLDialogElement>();
	let facteurs = $state<FacteursMoi | null>(null);
	let erreur = $state<ErreurApi | null>(null);
	let messagePasskey = $state('');
	let enCours = $state(false);
	let motDePasse = $state('');
	let code = $state('');

	$effect(() => {
		if (dialogue === undefined) return;
		if (reauthentification.ouverte && !dialogue.open) {
			erreur = null;
			messagePasskey = '';
			facteurs = null;
			dialogue.showModal();
			lireFacteurs(api).then(
				(lus) => (facteurs = lus),
				(e: unknown) => {
					if (e instanceof ErreurApi) erreur = e;
				}
			);
		} else if (!reauthentification.ouverte && dialogue.open) {
			dialogue.close();
		}
	});

	async function reussir(): Promise<void> {
		motDePasse = '';
		code = '';
		await invalidate('sigma:session');
		reauthentification.terminer(true);
	}

	async function parSecours(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		enCours = true;
		erreur = null;
		try {
			await reauthentifierParSecours(api, motDePasse, code);
			await reussir();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
			code = '';
		} finally {
			enCours = false;
		}
	}

	async function parPasskey(): Promise<void> {
		enCours = true;
		erreur = null;
		messagePasskey = '';
		try {
			const { defi } = await defiReauthentificationPasskey(api);
			await reauthentifierParPasskey(api, await authentifier(defi));
			await reussir();
		} catch (e) {
			if (e instanceof ErreurPasskey) {
				messagePasskey =
					e.raison === 'annulee'
						? 'La vérification a été annulée ou a expiré.'
						: 'Ce navigateur ne prend pas en charge les passkeys.';
			} else if (e instanceof ErreurApi) {
				erreur = e;
			} else {
				throw e;
			}
		} finally {
			enCours = false;
		}
	}

	function annuler(): void {
		motDePasse = '';
		code = '';
		reauthentification.terminer(false);
	}
</script>

<dialog
	bind:this={dialogue}
	aria-labelledby="titre-reauth"
	oncancel={(e) => {
		e.preventDefault();
		annuler();
	}}
>
	<h2 id="titre-reauth">Confirmez votre identité</h2>
	<p class="aide">Cette action sensible demande une vérification récente.</p>

	{#if erreur}
		<MessageErreur {erreur} />
	{/if}
	{#if messagePasskey}
		<div class="message message-avertissement" role="alert"><p>{messagePasskey}</p></div>
	{/if}

	{#if facteurs === null && erreur === null}
		<p role="status">Chargement…</p>
	{/if}

	{#if facteurs !== null && facteurs.passkeys.length > 0 && passkeysPrisesEnCharge()}
		<button type="button" class="bouton" onclick={parPasskey} disabled={enCours}>
			Utiliser ma passkey
		</button>
	{/if}

	{#if facteurs?.secours?.utilisable}
		<form onsubmit={parSecours}>
			<div class="champ">
				<label for="reauth-mdp">Mot de passe</label>
				<input
					id="reauth-mdp"
					type="password"
					autocomplete="current-password"
					required
					maxlength="1024"
					bind:value={motDePasse}
				/>
			</div>
			<div class="champ">
				<label for="reauth-code">Code de l’application d’authentification</label>
				<input
					id="reauth-code"
					type="text"
					inputmode="numeric"
					autocomplete="one-time-code"
					pattern="[0-9 ]*"
					required
					maxlength="12"
					bind:value={code}
				/>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>Confirmer</button>
		</form>
	{/if}

	<p>
		<button type="button" class="bouton bouton-secondaire" onclick={annuler}>Annuler</button>
	</p>
</dialog>

<style>
	dialog {
		max-width: min(30rem, calc(100vw - 2rem));
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
		padding: var(--espace-5);
	}

	dialog::backdrop {
		background: rgb(0 0 0 / 0.5);
	}
</style>
