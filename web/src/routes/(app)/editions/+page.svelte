<script lang="ts">
	import {
		FILTRE_VIDE,
		lireBalance,
		lireCentralisateur,
		lireGrandLivre,
		lireJournaux,
		type Balance,
		type Centralisateur,
		type EnteteEdition,
		type FiltreEdition,
		type GrandLivre,
		type Journaux,
		type NatureEdition
	} from '#lib/api/editions.ts';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { messageEdition } from '#lib/api/messages-editions.ts';
	import type { Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import FormulaireEdition from '#lib/composants/editions/FormulaireEdition.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import TableBalance from '#lib/composants/editions/TableBalance.svelte';
	import TableCentralisateur from '#lib/composants/editions/TableCentralisateur.svelte';
	import TableGrandLivre from '#lib/composants/editions/TableGrandLivre.svelte';
	import TableJournaux from '#lib/composants/editions/TableJournaux.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import {
		csvBalance,
		csvCentralisateur,
		csvGrandLivre,
		csvJournaux,
		nomFichierEdition
	} from '#lib/csv.ts';
	import { afficherDate } from '#lib/dates.ts';
	import {
		editionVide,
		LIBELLES_NATURE,
		messageBrouillonsExclus
	} from '#lib/editions-affichage.ts';
	import { enregistrerFichier } from '#lib/telechargement.ts';
	import '#lib/styles/editions.css';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	type Affichee =
		| { nature: 'balance'; edition: Balance }
		| { nature: 'grand-livre'; edition: GrandLivre }
		| { nature: 'journaux'; edition: Journaux }
		| { nature: 'centralisateur'; edition: Centralisateur };
	type Etat =
		| { phase: 'repos' }
		| { phase: 'chargement' }
		| { phase: 'erreur'; erreur: ErreurApi }
		| { phase: 'ok'; affichee: Affichee };

	let nature = $derived<NatureEdition>(data.nature);
	let filtre = $state<FiltreEdition>({ ...FILTRE_VIDE });
	let etat = $state<Etat>({ phase: 'repos' });
	let erreurExport = $state<string | null>(null);
	/** Numéro de la dernière demande : une réponse tardive d'une demande périmée est ignorée. */
	let demande = 0;

	// Un autre dossier dans l'URL (retour arrière) : l'édition affichée est celle de l'ancien
	// dossier. Elle est retirée, les réponses en vol sont périmées et le filtre est remis à zéro.
	const cleDossier = $derived(
		data.contenu?.ok
			? `${data.contenu.valeur.contexte.cabinet}/${data.contenu.valeur.contexte.dossier}`
			: ''
	);
	$effect.pre(() => {
		void cleDossier;
		demande += 1;
		etat = { phase: 'repos' };
		erreurExport = null;
		filtre = { ...FILTRE_VIDE };
	});

	async function afficher(cabinet: Uuid, dossier: Uuid): Promise<void> {
		const numero = ++demande;
		const demandee = nature;
		etat = { phase: 'chargement' };
		erreurExport = null;
		try {
			const f = $state.snapshot(filtre);
			let affichee: Affichee;
			if (demandee === 'balance') {
				affichee = { nature: 'balance', edition: await lireBalance(api, cabinet, dossier, f) };
			} else if (demandee === 'grand-livre') {
				affichee = {
					nature: 'grand-livre',
					edition: await lireGrandLivre(api, cabinet, dossier, f)
				};
			} else if (demandee === 'journaux') {
				affichee = { nature: 'journaux', edition: await lireJournaux(api, cabinet, dossier, f) };
			} else {
				affichee = {
					nature: 'centralisateur',
					edition: await lireCentralisateur(api, cabinet, dossier, f)
				};
			}
			if (numero === demande) etat = { phase: 'ok', affichee };
		} catch (erreur) {
			if (!(erreur instanceof ErreurApi)) throw erreur;
			if (numero === demande) etat = { phase: 'erreur', erreur };
		}
	}

	function csvDe(a: Affichee): string {
		switch (a.nature) {
			case 'balance':
				return csvBalance(a.edition);
			case 'grand-livre':
				return csvGrandLivre(a.edition);
			case 'journaux':
				return csvJournaux(a.edition);
			case 'centralisateur':
				return csvCentralisateur(a.edition);
		}
	}

	function exporter(a: Affichee): void {
		erreurExport = null;
		const entete: EnteteEdition = a.edition;
		try {
			enregistrerFichier(
				new Blob([csvDe(a)], { type: 'text/csv;charset=utf-8' }),
				nomFichierEdition(a.nature, entete.exercice, entete.mode)
			);
		} catch {
			erreurExport = 'Le fichier n’a pas pu être préparé.';
		}
	}
</script>

<svelte:head>
	<title>Éditions — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Éditions</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Les éditions n’ont pas pu être chargées." />
{:else}
	{@const { contexte, valeur } = data.contenu.valeur}
	<h1>Éditions — {contexte.nomDossier.nom}</h1>
	<NavigationDossier
		cabinet={contexte.cabinet}
		dossier={contexte.dossier}
		actif="editions"
		role={contexte.membre.role}
	/>

	<FormulaireEdition
		bind:nature
		bind:filtre
		exercices={valeur.exercices}
		journaux={valeur.journaux}
		enCours={etat.phase === 'chargement'}
		surAfficher={() => void afficher(contexte.cabinet, contexte.dossier)}
	/>

	<section
		aria-labelledby="titre-resultat"
		aria-live="polite"
		aria-busy={etat.phase === 'chargement'}
	>
		<h2 id="titre-resultat" class="sr-only">Résultat de l’édition</h2>
		{#if etat.phase === 'repos'}
			<p class="aide">
				Choisissez une édition et ses filtres, puis affichez-la. Le serveur calcule les totaux et
				les soldes ; l’édition est lue en entier (sans pagination).
			</p>
		{:else if etat.phase === 'chargement'}
			<p role="status">Calcul de l’édition en cours…</p>
		{:else if etat.phase === 'erreur'}
			<div class="message message-erreur" role="alert">
				<p>{messageEdition(etat.erreur)}</p>
			</div>
		{:else}
			{@const a = etat.affichee}
			{@const exclus = messageBrouillonsExclus(a.edition.brouillons_exclus)}
			<h3>
				{LIBELLES_NATURE[a.nature]} — exercice {a.edition.exercice}, du
				{afficherDate(a.edition.du)} au {afficherDate(a.edition.au)}
				<span class="badge">{a.edition.mode === 'provisoire' ? 'PROVISOIRE' : 'Définitive'}</span>
			</h3>
			{#if a.edition.mode === 'provisoire'}
				<div class="message message-avertissement" role="status">
					<p>
						Édition <strong>provisoire</strong> : elle inclut le brouillard équilibré (écritures non validées,
						sans numéro de pièce). Elle n’a aucune valeur comptable définitive.
					</p>
				</div>
			{/if}
			{#if exclus !== null}
				<div class="message message-avertissement" role="alert">
					<p>{exclus}</p>
				</div>
			{/if}
			{#if editionVide(a)}
				<p>Aucune écriture pour ces filtres.</p>
			{:else}
				<div class="barre-actions">
					<button class="bouton bouton-secondaire" type="button" onclick={() => exporter(a)}>
						Exporter en CSV
					</button>
					<button class="bouton bouton-secondaire" type="button" onclick={() => window.print()}>
						Imprimer
					</button>
				</div>
				{#if erreurExport !== null}
					<div class="message message-erreur" role="alert"><p>{erreurExport}</p></div>
				{/if}
				{#if a.nature === 'balance'}
					<TableBalance balance={a.edition} />
				{:else if a.nature === 'grand-livre'}
					<TableGrandLivre grandLivre={a.edition} />
				{:else if a.nature === 'journaux'}
					<TableJournaux journaux={a.edition} />
				{:else}
					<TableCentralisateur centralisateur={a.edition} />
				{/if}
			{/if}
		{/if}
	</section>
{/if}
