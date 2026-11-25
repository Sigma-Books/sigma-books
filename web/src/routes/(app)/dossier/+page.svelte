<script lang="ts">
	import { resolve } from '$app/paths';
	import { ROLES_INTEGRITE } from '#lib/api/integrite.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { libelleRole } from '#lib/libelles.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();
</script>

<svelte:head>
	<title>Dossier — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Dossier</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Le dossier n’a pas pu être chargé." />
{:else}
	{@const { membre, dossier } = data.contenu.valeur}
	<h1>{dossier.nom}</h1>
	<dl class="fiche">
		<dt>Cabinet</dt>
		<dd>{membre.nom}</dd>
		<dt>Votre rôle</dt>
		<dd>{libelleRole(membre.role)}</dd>
	</dl>
	<ul class="ecrans">
		<li>
			<a href={resolve(`saisie/?cabinet=${membre.id}&dossier=${dossier.id}`)}>Saisie manuelle</a>
		</li>
		<li>
			<a href={resolve(`brouillard/?cabinet=${membre.id}&dossier=${dossier.id}`)}>Brouillard</a>
		</li>
		<li>
			<a href={resolve(`ecritures/?cabinet=${membre.id}&dossier=${dossier.id}`)}>
				Écritures validées
			</a>
		</li>
		<li>
			<a href={resolve(`editions/?cabinet=${membre.id}&dossier=${dossier.id}`)}>Éditions</a>
		</li>
		{#if ROLES_INTEGRITE.includes(membre.role)}
			<li>
				<a href={resolve(`integrite/?cabinet=${membre.id}&dossier=${dossier.id}`)}>Intégrité</a>
			</li>
		{/if}
		<li>
			<a href={resolve(`alertes/?cabinet=${membre.id}&dossier=${dossier.id}`)}>Alertes</a>
		</li>
	</ul>
{/if}

<style>
	.fiche {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: var(--espace-2) var(--espace-4);
		margin: 0 0 var(--espace-5);
	}

	.fiche dt {
		font-weight: 600;
	}

	.fiche dd {
		margin: 0;
	}
</style>
