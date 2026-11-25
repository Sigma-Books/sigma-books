<!--
	Méthodes de connexion de l'utilisateur : passkeys, méthode de secours (mot de passe et TOTP),
	codes de récupération. Une session restreinte ou d'enrôlement (code de récupération, lien reçu
	par courriel) ne voit que l'enrôlement ; le serveur décide de ce qu'elle peut faire.
-->
<script lang="ts">
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		defiEnrolementPasskey,
		enrolerPasskey,
		enrolerSecours,
		lireFacteurs,
		preparerSecours,
		regenererCodes,
		supprimerPasskey
	} from '#lib/api/ressources.ts';
	import type { FacteursMoi, PasskeyListee, PreparationSecours } from '#lib/api/types.ts';
	import { api, seDeconnecter } from '#lib/application.ts';
	import CodesRecuperation from '#lib/composants/CodesRecuperation.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { dateHeure } from '#lib/instants.ts';
	import { grouperSecret, uriOtpauthSure } from '#lib/libelles.ts';
	import { enroler, ErreurPasskey, passkeysPrisesEnCharge } from '#lib/passkeys.ts';
	import { assurerFraicheur } from '#lib/reauthentification.svelte.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const complete = $derived(data.session.ok && data.session.valeur.nature === 'complete');

	let facteurs = $state<FacteursMoi | null>(null);
	let erreurChargement = $state<ErreurApi | null>(null);
	let erreur = $state<ErreurApi | null>(null);
	let avis = $state('');
	let enCours = $state(false);
	let nomPasskey = $state('');
	let preparation = $state<PreparationSecours | null>(null);
	let motDePasse = $state('');
	let code = $state('');
	let codes = $state<string[] | null>(null);
	let enrolementFait = $state(false);

	async function charger(): Promise<void> {
		erreurChargement = null;
		try {
			facteurs = await lireFacteurs(api);
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreurChargement = e;
		}
	}

	$effect(() => {
		if (complete) void charger();
	});

	/** Action sensible : ré-authentification d'abord (session complète), puis l'action. */
	async function agir(
		travail: () => Promise<void>,
		{ sensible = true }: { sensible?: boolean } = {}
	): Promise<void> {
		enCours = true;
		erreur = null;
		avis = '';
		try {
			if (sensible && complete && !(await assurerFraicheur())) return;
			await travail();
		} catch (e) {
			if (e instanceof ErreurPasskey) {
				avis =
					e.raison === 'annulee'
						? 'L’enregistrement a été annulé ou a expiré.'
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

	async function recharger(): Promise<void> {
		if (complete) await charger();
		else enrolementFait = true;
	}

	function ajouterPasskey(evenement: SubmitEvent): void {
		evenement.preventDefault();
		void agir(async () => {
			const { defi } = await defiEnrolementPasskey(api);
			const credential = await enroler(defi);
			await enrolerPasskey(api, nomPasskey, credential);
			nomPasskey = '';
			avis = 'Passkey enregistrée. Vos autres sessions ont été fermées.';
			await recharger();
		});
	}

	function retirerPasskey(passkey: PasskeyListee): void {
		void agir(async () => {
			await supprimerPasskey(api, passkey.id);
			avis = 'Passkey supprimée. Vos autres sessions ont été fermées.';
			await charger();
		});
	}

	function commencerSecours(): void {
		void agir(async () => {
			preparation = await preparerSecours(api);
		});
	}

	function confirmerSecours(evenement: SubmitEvent): void {
		evenement.preventDefault();
		const prepare = preparation;
		if (prepare === null) return;
		void agir(
			async () => {
				try {
					codes = (await enrolerSecours(api, prepare.jeton_enrolement, motDePasse, code))
						.codes_recuperation;
					preparation = null;
					avis = 'Méthode de secours enregistrée. Vos autres sessions ont été fermées.';
					await recharger();
				} finally {
					motDePasse = '';
					code = '';
				}
			},
			{ sensible: false }
		);
	}

	function nouvellesCodes(): void {
		void agir(async () => {
			codes = (await regenererCodes(api)).codes_recuperation;
			avis = 'Nouvelle série créée : l’ancienne est annulée.';
			await charger();
		});
	}

	function annulerSecours(): void {
		preparation = null;
		motDePasse = '';
		code = '';
	}

	let erreurDeconnexion = $state<ErreurApi | null>(null);
	async function deconnecter(): Promise<void> {
		try {
			await seDeconnecter();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreurDeconnexion = e;
		}
	}
</script>

<svelte:head>
	<title>Méthodes de connexion — Sigma Books</title>
</svelte:head>

<h1>Méthodes de connexion</h1>

{#if !complete}
	<div class="message message-avertissement" role="status">
		<p>
			Cette session ne sert qu’à enregistrer une méthode de connexion : une passkey (recommandé) ou
			la méthode de secours. Ensuite, déconnectez-vous et connectez-vous avec elle.
		</p>
	</div>
{/if}

<div role="status" aria-live="polite">
	{#if avis}<div class="message message-succes"><p>{avis}</p></div>{/if}
</div>
{#if erreur}
	<MessageErreur {erreur} contexte="L’opération a échoué." />
{/if}
{#if erreurDeconnexion}
	<MessageErreur erreur={erreurDeconnexion} contexte="La déconnexion a échoué." />
{/if}

{#if codes !== null}
	<CodesRecuperation {codes} termine={() => (codes = null)} />
{/if}

{#if enrolementFait && !complete}
	<p>
		<button type="button" class="bouton" onclick={deconnecter}>
			Se déconnecter pour se connecter avec la nouvelle méthode
		</button>
	</p>
{/if}

{#if complete}
	{#if erreurChargement}
		<MessageErreur erreur={erreurChargement} contexte="Vos méthodes n’ont pas pu être chargées." />
	{:else if facteurs === null}
		<p role="status">Chargement…</p>
	{/if}
{/if}

<section aria-labelledby="titre-passkeys">
	<h2 id="titre-passkeys">Passkeys</h2>
	{#if facteurs !== null}
		{#if facteurs.passkeys.length === 0}
			<p>Aucune passkey enregistrée.</p>
		{:else}
			<ul class="liste">
				{#each facteurs.passkeys as passkey (passkey.id)}
					<li class="element">
						<div>
							<strong>{passkey.nom}</strong>
							<span class="aide">
								créée le {dateHeure(passkey.cree_le)}, {passkey.derniere_utilisation
									? `utilisée le ${dateHeure(passkey.derniere_utilisation)}`
									: 'jamais utilisée'}
							</span>
						</div>
						<button
							type="button"
							class="bouton bouton-secondaire"
							disabled={enCours}
							onclick={() => retirerPasskey(passkey)}
							aria-label={`Supprimer la passkey ${passkey.nom}`}
						>
							Supprimer
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	{/if}

	{#if passkeysPrisesEnCharge()}
		<form onsubmit={ajouterPasskey}>
			<div class="champ">
				<label for="nom-passkey">Nom de la nouvelle passkey</label>
				<input
					id="nom-passkey"
					type="text"
					required
					maxlength="100"
					autocomplete="off"
					aria-describedby="aide-nom-passkey"
					bind:value={nomPasskey}
				/>
				<span id="aide-nom-passkey" class="aide">Par exemple : « Téléphone de Awa ».</span>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>Ajouter une passkey</button>
		</form>
	{:else}
		<p class="aide">Ce navigateur ne prend pas en charge les passkeys.</p>
	{/if}
</section>

<section aria-labelledby="titre-secours">
	<h2 id="titre-secours">Méthode de secours</h2>
	{#if facteurs !== null}
		{#if facteurs.secours === null}
			<p>Aucune méthode de secours enregistrée.</p>
		{:else if !facteurs.secours.utilisable}
			<div class="message message-avertissement" role="alert">
				<p>La méthode de secours doit être enregistrée à nouveau (clé du serveur changée).</p>
			</div>
		{:else}
			<p>Mot de passe et code TOTP, enregistrés le {dateHeure(facteurs.secours.modifie_le)}.</p>
		{/if}
		<p class="aide">Codes de récupération restants : {facteurs.codes_restants}.</p>
	{/if}

	{#if preparation === null}
		<button
			type="button"
			class="bouton bouton-secondaire"
			disabled={enCours}
			onclick={commencerSecours}
		>
			{facteurs?.secours ? 'Remplacer la méthode de secours' : 'Configurer la méthode de secours'}
		</button>
		{#if complete && facteurs?.secours}
			<button
				type="button"
				class="bouton bouton-secondaire"
				disabled={enCours}
				onclick={nouvellesCodes}
			>
				Générer de nouveaux codes de récupération
			</button>
		{/if}
	{:else}
		{@const lien = uriOtpauthSure(preparation.uri)}
		<form onsubmit={confirmerSecours}>
			<p>
				Ajoutez ce secret dans votre application d’authentification (saisie manuelle), puis
				saisissez un code. Choisissez un mot de passe d’au moins 12 caractères.
			</p>
			<p><code class="secret">{grouperSecret(preparation.secret)}</code></p>
			{#if lien}
				<!-- Schéma otpauth:// vérifié par uriOtpauthSure : lien vers une application locale. -->
				<!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
				<p><a href={lien}>Ouvrir dans l’application d’authentification</a></p>
			{/if}
			<div class="champ">
				<label for="nouveau-mdp">Mot de passe (12 caractères au moins)</label>
				<input
					id="nouveau-mdp"
					type="password"
					autocomplete="new-password"
					required
					minlength="12"
					maxlength="128"
					bind:value={motDePasse}
				/>
			</div>
			<div class="champ">
				<label for="code-totp">Code à 6 chiffres</label>
				<input
					id="code-totp"
					type="text"
					inputmode="numeric"
					autocomplete="one-time-code"
					pattern="[0-9 ]*"
					required
					maxlength="12"
					bind:value={code}
				/>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>Enregistrer</button>
			<button type="button" class="bouton bouton-secondaire" onclick={annulerSecours}>
				Annuler
			</button>
		</form>
	{/if}
</section>

<style>
	section {
		margin-top: var(--espace-6);
	}

	.element {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--espace-2);
		padding: var(--espace-3) var(--espace-4);
	}

	.element .aide {
		display: block;
	}

	.secret {
		font-family: ui-monospace, 'Cascadia Mono', Menlo, monospace;
		font-size: var(--taille-titre-2);
		overflow-wrap: anywhere;
	}
</style>
