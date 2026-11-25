<script lang="ts">
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		contrePasser,
		lireEcriture,
		type ContrePassation,
		type Ecriture
	} from '#lib/api/brouillard.ts';
	import { longueurOctets } from '#lib/api/texte.ts';
	import { lireUuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import MessageMetier from '#lib/composants/MessageMetier.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import { ROLES_VALIDATION_LOT } from '#lib/contexte-dossier.ts';
	import { afficherDate, dateIso } from '#lib/dates.ts';
	import { formaterMontant } from '#lib/montants.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	/** Écriture relue après une contre-passation (sa liste de contre-passations avance). */
	let relue = $state<Ecriture | null>(null);
	let faite = $state<ContrePassation | null>(null);
	let date = $state(dateIso(new Date()));
	let libelle = $state('');
	let motif = $state('');
	let confirmeRepetition = $state(false);
	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let erreurRelecture = $state<ErreurApi | null>(null);

	/** Borne du serveur, en octets UTF-8 (`MOTIF_MAX` de `contre_passation.rs`). */
	const MOTIF_MAX = 400;
	const octetsMotif = $derived(longueurOctets(motif));
	/** Contre-passations faites depuis le chargement, tant que la relecture n'a pas abouti. */
	let faitesDepuis = $state(0);

	$effect(() => {
		relue = null;
		faite = null;
		faitesDepuis = 0;
		if (data.contenu?.ok) {
			libelle = `Contre-passation de ${data.contenu.valeur.valeur.numero}`;
		}
	});

	async function contrePasserCette(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		if (!data.contenu?.ok) return;
		const { cabinet, dossier } = data.contenu.valeur.contexte;
		const ecriture = relue ?? data.contenu.valeur.valeur;
		enCours = true;
		erreur = null;
		erreurRelecture = null;
		try {
			faite = await contrePasser(api, cabinet, dossier, ecriture.id, { date, libelle, motif });
			faitesDepuis += 1;
			motif = '';
			confirmeRepetition = false;
			try {
				relue = await lireEcriture(api, cabinet, dossier, ecriture.id);
			} catch (e) {
				if (!(e instanceof ErreurApi)) throw e;
				erreurRelecture = e;
			}
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = false;
		}
	}
</script>

<svelte:head>
	<title>Écriture validée — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Écriture validée</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="L’écriture n’a pas pu être chargée." />
{:else}
	{@const { contexte, valeur } = data.contenu.valeur}
	{@const e = relue ?? valeur}
	{@const nombre =
		relue === null
			? valeur.contre_passations.length + faitesDepuis
			: relue.contre_passations.length}
	{@const origine = e.origine ? lireUuid(e.origine) : null}
	{@const peut = ROLES_VALIDATION_LOT.includes(contexte.membre.role)}
	<h1>Écriture {e.numero}</h1>
	<NavigationDossier cabinet={contexte.cabinet} dossier={contexte.dossier} actif="ecritures" />

	<dl class="fiche">
		<dt>Libellé</dt>
		<dd>{e.libelle}</dd>
		<dt>Journal</dt>
		<dd>{e.journal} (exercice {e.exercice})</dd>
		<dt>Date comptable</dt>
		<dd>{afficherDate(e.date)}</dd>
		{#if e.date_valeur_comptable}
			<dt>Date de valeur</dt>
			<dd>{afficherDate(e.date_valeur_comptable)}</dd>
		{/if}
		<dt>Validée le</dt>
		<dd>{afficherDate(e.date_validation)}</dd>
		<dt>Rang dans la chaîne</dt>
		<dd>{e.rang}</dd>
		<dt>Empreinte</dt>
		<dd><code class="empreinte">{e.empreinte}</code></dd>
		{#if origine !== null}
			<dt>Contre-passation de</dt>
			<dd>
				<a
					href={resolve(
						`ecriture/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}&ecriture=${origine}`
					)}
				>
					l’écriture d’origine
				</a>
			</dd>
		{/if}
	</dl>

	<h2>Lignes</h2>
	<div class="defilement">
		<table>
			<caption class="sr-only">Lignes de l’écriture {e.numero}</caption>
			<thead>
				<tr>
					<th scope="col">N°</th>
					<th scope="col">Compte</th>
					<th scope="col" class="num">Débit</th>
					<th scope="col" class="num">Crédit</th>
				</tr>
			</thead>
			<tbody>
				{#each e.lignes as l (l.position)}
					<tr>
						<th scope="row">{l.position}</th>
						<td>{l.compte}</td>
						<td class="num">{l.sens === 'debit' ? formaterMontant(l.montant) : ''}</td>
						<td class="num">{l.sens === 'credit' ? formaterMontant(l.montant) : ''}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>

	<h2>Pièces justificatives</h2>
	{#if e.pieces.length === 0}
		<p class="aide">Aucune pièce liée.</p>
	{:else}
		<ul>
			{#each e.pieces as p (p.piece)}
				<li>
					Pièce {p.role === 'principale' ? 'principale' : 'complémentaire'}
					<code>{p.piece.slice(0, 8)}</code> — empreinte
					<code class="empreinte">{p.empreinte}</code>
				</li>
			{/each}
		</ul>
	{/if}

	<h2>Contre-passations</h2>
	{#if nombre > 0}
		<div class="message message-avertissement" role="status">
			<p>
				<strong>Déjà contre-passée {nombre} fois.</strong>
				Une nouvelle contre-passation annulerait l’écriture une fois de plus (son effet serait doublé).
			</p>
		</div>
		<ul>
			{#each e.contre_passations as numero (numero)}
				<li>{numero}</li>
			{/each}
		</ul>
	{:else}
		<p class="aide">Cette écriture n’a pas été contre-passée.</p>
	{/if}

	{#if faite}
		<div class="message message-succes" role="status">
			<p>
				Contre-passation validée sous le numéro
				<a
					href={resolve(
						`ecriture/?cabinet=${contexte.cabinet}&dossier=${contexte.dossier}&ecriture=${faite.id}`
					)}
				>
					{faite.numero}
				</a>.
			</p>
		</div>
	{/if}
	{#if erreurRelecture}
		<MessageErreur erreur={erreurRelecture} contexte="La liste n’a pas pu être relue." />
	{/if}

	{#if peut}
		<form onsubmit={contrePasserCette} aria-labelledby="titre-cp">
			<h3 id="titre-cp">Contre-passer cette écriture</h3>
			<p class="aide">
				La contre-passation crée une nouvelle écriture validée qui inverse le débit et le crédit de
				chaque ligne. L’écriture d’origine reste inchangée.
			</p>
			<div class="defilement">
				<table>
					<caption class="sr-only">
						Lignes de la contre-passation (débit et crédit inversés)
					</caption>
					<thead>
						<tr>
							<th scope="col">Compte</th>
							<th scope="col" class="num">Débit</th>
							<th scope="col" class="num">Crédit</th>
						</tr>
					</thead>
					<tbody>
						{#each e.lignes as l (l.position)}
							<tr>
								<td>{l.compte}</td>
								<td class="num">{l.sens === 'credit' ? formaterMontant(l.montant) : ''}</td>
								<td class="num">{l.sens === 'debit' ? formaterMontant(l.montant) : ''}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
			<div class="champ">
				<label for="cp-date">Date de la contre-passation</label>
				<input id="cp-date" type="date" bind:value={date} required />
			</div>
			<div class="champ">
				<label for="cp-libelle">Libellé</label>
				<input id="cp-libelle" type="text" maxlength="200" bind:value={libelle} required />
			</div>
			<div class="champ">
				<label for="cp-motif">Motif (obligatoire, conservé au journal d’audit)</label>
				<input
					id="cp-motif"
					type="text"
					aria-describedby="cp-motif-aide"
					bind:value={motif}
					required
				/>
				<span class="aide" id="cp-motif-aide">
					{octetsMotif} / {MOTIF_MAX} octets{octetsMotif > MOTIF_MAX
						? ' : motif trop long, raccourcissez-le'
						: ''}
				</span>
			</div>
			{#if nombre > 0}
				<label class="case">
					<input type="checkbox" bind:checked={confirmeRepetition} />
					Je confirme vouloir contre-passer une écriture déjà contre-passée {nombre} fois.
				</label>
			{/if}
			{#if erreur}
				<MessageMetier {erreur} contexte="La contre-passation a échoué." />
			{/if}
			<p>
				<button
					type="submit"
					class="bouton"
					disabled={enCours ||
						motif.trim() === '' ||
						octetsMotif > MOTIF_MAX ||
						(nombre > 0 && !confirmeRepetition)}
				>
					Contre-passer
				</button>
			</p>
		</form>
	{/if}
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
		min-width: 0;
	}

	.empreinte {
		overflow-wrap: anywhere;
	}

	.defilement {
		overflow-x: auto;
		margin-bottom: var(--espace-4);
	}

	table {
		width: 100%;
		min-width: 24rem;
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

	.case {
		display: flex;
		align-items: center;
		gap: var(--espace-2);
		min-height: var(--cible-min);
		margin-bottom: var(--espace-3);
	}

	input[type='date'],
	input[type='text'] {
		min-height: var(--cible-min);
		padding: var(--espace-2) var(--espace-3);
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
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
