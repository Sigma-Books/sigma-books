<script lang="ts">
	import { replaceState } from '$app/navigation';
	import { resolve } from '$app/paths';
	import type { Brouillon } from '#lib/api/brouillard.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import EditeurEcriture from '#lib/composants/saisie/EditeurEcriture.svelte';
	import PiecesBrouillon from '#lib/composants/saisie/PiecesBrouillon.svelte';
	import { ROLES_SAISIE } from '#lib/contexte-dossier.ts';
	import { afficherDate } from '#lib/dates.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	/** Brouillon ouvert (en mode « par pièce ») : il porte les pièces et la version courante. */
	let ouvert = $state<Brouillon | null>(null);
	/** Écritures enregistrées pendant la session de saisie au kilomètre. */
	let enregistrees = $state<Brouillon[]>([]);

	$effect(() => {
		ouvert = data.contenu?.ok ? data.contenu.valeur.valeur.ouvert : null;
		enregistrees = [];
	});

	function sansBrouillon(): void {
		const contexte = data.contenu?.ok ? data.contenu.valeur.contexte : null;
		if (contexte === null || data.mode !== 'piece') return;
		ouvert = null;
		replaceState(resolve(`saisie/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}`), {});
	}

	function surEnregistre(b: Brouillon, creation: boolean): void {
		const contexte = data.contenu?.ok ? data.contenu.valeur.contexte : null;
		if (contexte === null) return;
		if (data.mode === 'kilometre') {
			if (creation) enregistrees = [b, ...enregistrees];
			return;
		}
		ouvert = b;
		if (creation) {
			// L'adresse désigne désormais le brouillon : un rechargement le rouvre.
			replaceState(
				resolve(
					`saisie/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}&brouillon=${b.id}`
				),
				{}
			);
		}
	}
</script>

<svelte:head>
	<title>Saisie — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Saisie</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="La saisie n’a pas pu être chargée." />
{:else}
	{@const { contexte, valeur } = data.contenu.valeur}
	<h1>Saisie manuelle — {contexte.nomDossier.nom}</h1>
	<NavigationDossier cabinet={contexte.cabinet} dossier={contexte.dossier} actif="saisie" />

	{#if !ROLES_SAISIE.includes(contexte.membre.role)}
		<div class="message message-avertissement" role="status">
			<p>Votre rôle ne permet pas de saisir des écritures. Le serveur refusera l’enregistrement.</p>
		</div>
	{/if}

	<p class="modes" role="group" aria-label="Mode de saisie">
		<a
			class="bouton"
			class:bouton-secondaire={data.mode !== 'piece'}
			aria-current={data.mode === 'piece' ? 'true' : undefined}
			href={resolve(`saisie/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}`)}
		>
			Par pièce
		</a>
		<a
			class="bouton"
			class:bouton-secondaire={data.mode !== 'kilometre'}
			aria-current={data.mode === 'kilometre' ? 'true' : undefined}
			href={resolve(
				`saisie/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}&mode=kilometre`
			)}
		>
			Au kilomètre
		</a>
	</p>
	<p class="aide">
		{#if data.mode === 'kilometre'}
			Saisie au kilomètre : après chaque enregistrement, le journal et la date sont conservés et une
			nouvelle écriture vide s’ouvre. Joignez les pièces plus tard, depuis le brouillard.
		{:else}
			Saisie par pièce : une écriture à la fois, avec ses pièces justificatives. Le montant en
			devise et l’étiquette fiscale (TVA) ne sont pas saisissables ici.
		{/if}
	</p>

	{#key `${data.mode}-${data.contenu.valeur.valeur.ouvert?.id ?? 'neuf'}`}
		<EditeurEcriture
			cabinet={contexte.cabinet}
			dossier={contexte.dossier}
			comptes={valeur.comptes}
			journaux={valeur.journaux}
			tiers={valeur.tiers}
			mode={data.mode}
			brouillon={data.mode === 'piece' ? ouvert : null}
			{surEnregistre}
			surNouvelle={sansBrouillon}
		/>
	{/key}

	{#if data.mode === 'piece' && ouvert !== null}
		<PiecesBrouillon
			cabinet={contexte.cabinet}
			dossier={contexte.dossier}
			brouillon={ouvert}
			surChangement={(b) => (ouvert = b)}
		/>
		<p>
			<a href={resolve(`brouillard/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}`)}>
				Aller au brouillard pour valider
			</a>
		</p>
	{/if}

	{#if data.mode === 'kilometre' && enregistrees.length > 0}
		<section aria-labelledby="titre-enregistrees">
			<h2 id="titre-enregistrees">Enregistrées pendant cette session ({enregistrees.length})</h2>
			<ul class="enregistrees">
				{#each enregistrees as b (b.id)}
					<li>
						{afficherDate(b.date)} · {b.journal} · {b.libelle}
						<span class="aide">({b.lignes.length} lignes)</span>
						<a
							href={resolve(
								`saisie/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}&brouillon=${b.id}`
							)}
						>
							Rouvrir avec ses pièces
						</a>
					</li>
				{/each}
			</ul>
		</section>
	{/if}
{/if}

<style>
	.modes {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-2);
	}

	.modes a {
		text-decoration: none;
	}

	.enregistrees {
		padding-left: var(--espace-4);
	}
</style>
