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
	import IdentiteDossier from '#lib/composants/IdentiteDossier.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import {
		csvBalance,
		csvCentralisateur,
		csvGrandLivre,
		csvJournaux,
		nomFichierEdition,
		OPTIONS_CSV_SANS_DOSSIER,
		optionsCsvParDefaut,
		SEPARATEURS_CSV,
		SEPARATEURS_DECIMAUX,
		type ContexteCsv,
		type OptionsCsv
	} from '#lib/csv.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { editionVide, libelleEdition, messageBrouillonsExclus } from '#lib/editions-affichage.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import type { Exercice } from '#lib/api/referentiel.ts';
	import type { Regional } from '#lib/regional.ts';
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
	/** Options du CSV : celles du locale du dossier tant que l'utilisateur n'y touche pas. */
	let optionsCsv = $state<OptionsCsv>({ ...OPTIONS_CSV_SANS_DOSSIER });
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
	$effect.pre(() => {
		if (data.contenu?.ok) optionsCsv = optionsCsvParDefaut(data.contenu.valeur.contexte.regional);
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

	function csvDe(a: Affichee, c: ContexteCsv): string {
		switch (a.nature) {
			case 'balance':
				return csvBalance(a.edition, c);
			case 'grand-livre':
				return csvGrandLivre(a.edition, c);
			case 'journaux':
				return csvJournaux(a.edition, c);
			case 'centralisateur':
				return csvCentralisateur(a.edition, c);
		}
	}

	function exporter(
		a: Affichee,
		nomDossier: string,
		regional: Regional,
		exercices: Exercice[]
	): void {
		erreurExport = null;
		const entete: EnteteEdition = a.edition;
		try {
			enregistrerFichier(
				new Blob(
					[csvDe(a, { nomDossier, regional, exercices, options: $state.snapshot(optionsCsv) })],
					{
						type: 'text/csv;charset=utf-8'
					}
				),
				nomFichierEdition(a.nature, entete.exercice, entete.mode)
			);
		} catch {
			erreurExport = traduire('edition.export_impossible');
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
		regional={contexte.regional}
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
			{@const exercice = valeur.exercices.find((x) => x.code === a.edition.exercice)}
			<h3>
				{libelleEdition(a.nature)} —
				{#if exercice}
					{traduire('edition.exercice_dates', {
						code: exercice.code,
						debut: afficherDate(exercice.debut, contexte.regional),
						fin: afficherDate(exercice.fin, contexte.regional)
					})}{:else}{traduire('edition.exercice_code', { code: a.edition.exercice })}{/if}, du
				{afficherDate(a.edition.du, contexte.regional)} au
				{afficherDate(a.edition.au, contexte.regional)}
				<span class="badge">{a.edition.mode === 'provisoire' ? 'PROVISOIRE' : 'Définitive'}</span>
			</h3>
			<IdentiteDossier nom={contexte.nomDossier.nom} devise={contexte.regional.devise} />
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
					<button
						class="bouton bouton-secondaire"
						type="button"
						onclick={() =>
							exporter(a, contexte.nomDossier.nom, contexte.regional, valeur.exercices)}
					>
						Exporter en CSV
					</button>
					<button class="bouton bouton-secondaire" type="button" onclick={() => window.print()}>
						Imprimer
					</button>
				</div>
				<details class="options-csv no-print">
					<summary>{traduire('csv.options')}</summary>
					<div class="grille-filtre">
						<div class="champ">
							<label for="csv-sep">{traduire('csv.option_separateur')}</label>
							<select id="csv-sep" bind:value={optionsCsv.separateur}>
								{#each SEPARATEURS_CSV as sep (sep)}
									<option value={sep}>
										{traduire(
											sep === ';'
												? 'csv.sep.pointvirgule'
												: sep === ','
													? 'csv.sep.virgule'
													: 'csv.sep.tabulation'
										)}
									</option>
								{/each}
							</select>
						</div>
						<div class="champ">
							<label for="csv-dec">{traduire('csv.option_decimal')}</label>
							<select id="csv-dec" bind:value={optionsCsv.separateurDecimal}>
								{#each SEPARATEURS_DECIMAUX as dec (dec)}
									<option value={dec}>
										{traduire(dec === ',' ? 'csv.dec.virgule' : 'csv.dec.point')}
									</option>
								{/each}
							</select>
						</div>
					</div>
					<label class="choix-bom">
						<input type="checkbox" bind:checked={optionsCsv.bom} />
						{traduire('csv.option_bom')}
					</label>
				</details>
				{#if erreurExport !== null}
					<div class="message message-erreur" role="alert"><p>{erreurExport}</p></div>
				{/if}
				{#if a.nature === 'balance'}
					<TableBalance balance={a.edition} regional={contexte.regional} />
				{:else if a.nature === 'grand-livre'}
					<TableGrandLivre grandLivre={a.edition} regional={contexte.regional} />
				{:else if a.nature === 'journaux'}
					<TableJournaux journaux={a.edition} regional={contexte.regional} />
				{:else}
					<TableCentralisateur centralisateur={a.edition} regional={contexte.regional} />
				{/if}
			{/if}
		{/if}
	</section>
{/if}
