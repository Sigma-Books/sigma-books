<script lang="ts">
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { listerEcritures, type ResumeEcriture } from '#lib/api/brouillard.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import { afficherDate } from '#lib/dates.ts';
	import { formaterMontant } from '#lib/montants.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let ajoutees = $state<ResumeEcriture[]>([]);
	let suivant = $state<string | null>(null);
	let erreurSuite = $state<ErreurApi | null>(null);

	$effect(() => {
		ajoutees = [];
		suivant = data.contenu?.ok ? (data.contenu.valeur.valeur.suivant ?? null) : null;
	});

	async function chargerSuite(): Promise<void> {
		if (!data.contenu?.ok || suivant === null) return;
		const { cabinet, dossier } = data.contenu.valeur.contexte;
		erreurSuite = null;
		try {
			const page = await listerEcritures(api, cabinet, dossier, suivant);
			ajoutees = [...ajoutees, ...page.ecritures];
			suivant = page.suivant ?? null;
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreurSuite = e;
		}
	}
</script>

<svelte:head>
	<title>Écritures validées — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Écritures validées</h1>
	<MessageErreur
		erreur={data.contenu.erreur}
		contexte="Les écritures n’ont pas pu être chargées."
	/>
{:else}
	{@const { contexte, valeur } = data.contenu.valeur}
	{@const toutes = [...valeur.ecritures, ...ajoutees]}
	<h1>Écritures validées — {contexte.nomDossier.nom}</h1>
	<NavigationDossier
		cabinet={contexte.cabinet}
		dossier={contexte.dossier}
		actif="ecritures"
		role={contexte.membre.role}
	/>

	{#if toutes.length === 0}
		<p>Aucune écriture validée pour l’instant.</p>
	{:else}
		<div class="defilement">
			<table>
				<caption class="sr-only">Écritures validées, par date</caption>
				<thead>
					<tr>
						<th scope="col">Numéro</th>
						<th scope="col">Date</th>
						<th scope="col">Libellé</th>
						<th scope="col" class="num">Total</th>
					</tr>
				</thead>
				<tbody>
					{#each toutes as e (e.id)}
						<tr>
							<td>
								<a
									href={resolve(
										`ecriture/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}&ecriture=${e.id}`
									)}
								>
									{e.numero}
								</a>
							</td>
							<td>{afficherDate(e.date)}</td>
							<td>{e.libelle}</td>
							<td class="num">{formaterMontant(e.total)}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		{#if erreurSuite}
			<MessageErreur erreur={erreurSuite} contexte="La suite n’a pas pu être chargée." />
		{/if}
		{#if suivant !== null}
			<p>
				<button type="button" class="bouton bouton-secondaire" onclick={chargerSuite}>
					Charger la suite
				</button>
			</p>
		{/if}
	{/if}
{/if}

<style>
	.defilement {
		overflow-x: auto;
		margin-bottom: var(--espace-4);
	}

	table {
		width: 100%;
		min-width: 30rem;
		border-collapse: collapse;
	}

	th,
	td {
		padding: var(--espace-2);
		text-align: left;
		border-bottom: 1px solid var(--couleur-separateur);
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
	}
</style>
