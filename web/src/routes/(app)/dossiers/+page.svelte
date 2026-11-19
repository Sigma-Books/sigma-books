<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { creerDossier } from '#lib/api/ressources.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { libelleRole, ROLES_CREATION_DOSSIER } from '#lib/libelles.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let nom = $state('');
	let creationEnCours = $state(false);
	let erreurCreation = $state<ErreurApi | null>(null);

	async function creer(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		const cabinet = data.cabinet;
		if (cabinet === null) return;
		creationEnCours = true;
		erreurCreation = null;
		try {
			const dossier = await creerDossier(api, cabinet, nom);
			await goto(resolve(`dossier/?cabinet=${cabinet}&dossier=${dossier.id}`));
		} catch (erreur) {
			if (!(erreur instanceof ErreurApi)) throw erreur;
			erreurCreation = erreur;
		} finally {
			creationEnCours = false;
		}
	}
</script>

<svelte:head>
	<title>Dossiers — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Dossiers</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Les dossiers n’ont pas pu être chargés." />
{:else}
	{@const { membre, dossiers } = data.contenu.valeur}
	<h1>Dossiers de {membre.nom}</h1>
	<p class="aide">Votre rôle : {libelleRole(membre.role)}</p>

	{#if dossiers.length === 0}
		<p>Aucun dossier ne vous est accessible dans ce cabinet.</p>
	{:else}
		<ul class="liste">
			{#each dossiers as dossier (dossier.id)}
				<li>
					<a
						class="liste-lien"
						href={resolve(`dossier/?cabinet=${membre.id}&dossier=${dossier.id}`)}
					>
						<span class="nom">{dossier.nom}</span>
					</a>
				</li>
			{/each}
		</ul>
	{/if}

	{#if ROLES_CREATION_DOSSIER.includes(membre.role)}
		<section aria-labelledby="titre-creation">
			<h2 id="titre-creation">Nouveau dossier</h2>
			{#if erreurCreation}
				<MessageErreur erreur={erreurCreation} contexte="Le dossier n’a pas été créé." />
			{/if}
			<form onsubmit={creer}>
				<div class="champ">
					<label for="nom-dossier">Nom du dossier</label>
					<input
						id="nom-dossier"
						name="nom"
						type="text"
						required
						maxlength="200"
						autocomplete="off"
						aria-describedby="aide-nom-dossier"
						bind:value={nom}
					/>
					<span id="aide-nom-dossier" class="aide">200 caractères au plus.</span>
				</div>
				<button type="submit" class="bouton" disabled={creationEnCours}>Créer le dossier</button>
			</form>
		</section>
	{/if}
{/if}
