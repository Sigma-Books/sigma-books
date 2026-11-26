<!--
	Liste du brouillard : filtres (journal, période), drapeau « à revoir », validation unitaire,
	validation par lot (`LOT_MAX` au plus, tout ou rien, ré-authentification), suppression.

	Le serveur décide de tout : l'interface n'affiche que ses réponses et les motifs stables de ses
	refus. Les totaux et l'équilibre affichés sont ceux qu'il renvoie.
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		changerDrapeau,
		listerBrouillard,
		LOT_MAX,
		supprimerBrouillon,
		validerBrouillon,
		validerParLot,
		type ElementLot,
		type PageBrouillard,
		type ResumeBrouillon
	} from '#lib/api/brouillard.ts';
	import { messageMotif } from '#lib/api/messages.ts';
	import type { Exercice, Journal } from '#lib/api/referentiel.ts';
	import { estCodeJournal, lireUuid, type Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import { executerSensible } from '#lib/reauthentification.svelte.ts';
	import { ROLES_SAISIE, ROLES_VALIDATION_LOT } from '#lib/contexte-dossier.ts';
	import {
		bandeauAvertissements,
		lireAvertissements,
		type BandeauAvertissements
	} from '#lib/api/avertissements.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { ecart, formaterMontant } from '#lib/montants.ts';
	import type { Regional } from '#lib/regional.ts';
	import MessageErreur from '../MessageErreur.svelte';
	import MessageMetier from '../MessageMetier.svelte';

	interface Props {
		cabinet: Uuid;
		dossier: Uuid;
		/** Devise, échelle et locale du dossier : montants et dates affichés. */
		regional: Regional;
		role: string;
		journaux: Journal[];
		exercices: Exercice[];
		initiale: PageBrouillard;
		filtre: { journal: string; periode: Uuid | null };
	}

	let { cabinet, dossier, regional, role, journaux, exercices, initiale, filtre }: Props = $props();

	// Le parent recrée la liste (`{#key}`) à chaque nouveau filtre : seule la valeur initiale compte.
	// svelte-ignore state_referenced_locally
	let lignes = $state<ResumeBrouillon[]>(initiale.brouillons);
	// svelte-ignore state_referenced_locally
	let suivant = $state<string | null>(initiale.suivant ?? null);
	let seulementARevoir = $state(false);
	let selection = $state<string[]>([]);
	let occupes = $state<string[]>([]);
	let erreurs = $state<Record<string, ErreurApi>>({});
	let validees = $state<{ id: string; numero: string }[]>([]);
	let info = $state<string | null>(null);
	/** Bandeau d'information (non bloquant) : avertissements de la dernière validation réussie. */
	let avertissements = $state<BandeauAvertissements | null>(null);
	let erreurListe = $state<ErreurApi | null>(null);
	let aSupprimer = $state<string | null>(null);

	type PhaseLot = 'aucune' | 'confirmation';
	let phase = $state<PhaseLot>('aucune');
	let enCoursLot = $state(false);
	let erreurLot = $state<ErreurApi | null>(null);
	let refusLot = $state<{ ecriture: string; code: string }[]>([]);

	const peutSaisir = $derived(ROLES_SAISIE.includes(role));
	const peutLot = $derived(ROLES_VALIDATION_LOT.includes(role));
	const affichees = $derived(seulementARevoir ? lignes.filter((l) => l.a_revoir) : lignes);
	const parId = $derived(new Map<string, ResumeBrouillon>(lignes.map((l) => [l.id, l])));

	function libelleDe(id: string): string {
		const l = parId.get(id);
		return l === undefined
			? 'Écriture inconnue'
			: `${afficherDate(l.date, regional)} · ${l.journal} · ${l.libelle}`;
	}

	// --- Filtres ---------------------------------------------------------------------------

	// svelte-ignore state_referenced_locally
	let journalChoisi = $state(filtre.journal);
	// svelte-ignore state_referenced_locally
	let periodeChoisie = $state<string>(filtre.periode ?? '');

	async function filtrer(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		let requete = `cabinet=${cabinet}&dossier=${dossier}`;
		if (estCodeJournal(journalChoisi)) requete += `&journal=${journalChoisi}`;
		const periode = lireUuid(periodeChoisie);
		if (periode !== null) requete += `&periode=${periode}`;
		await goto(resolve(`brouillard/?${requete}`));
	}

	async function chargerSuite(): Promise<void> {
		if (suivant === null) return;
		erreurListe = null;
		try {
			const page = await listerBrouillard(api, cabinet, dossier, {
				journal: filtre.journal,
				periode: filtre.periode,
				apres: suivant
			});
			const connus = new Set(lignes.map((l) => l.id));
			lignes = [...lignes, ...page.brouillons.filter((b) => !connus.has(b.id))];
			suivant = page.suivant ?? null;
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreurListe = e;
		}
	}

	// --- Actions sur un brouillon ----------------------------------------------------------

	function retirerDeLaListe(ids: readonly string[]): void {
		lignes = lignes.filter((l) => !ids.includes(l.id));
		selection = selection.filter((s) => !ids.includes(s));
	}

	async function surBrouillon(id: string, travail: () => Promise<void>): Promise<void> {
		occupes = [...occupes, id];
		delete erreurs[id];
		info = null;
		avertissements = null;
		validees = [];
		try {
			await travail();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreurs[id] = e;
		} finally {
			occupes = occupes.filter((o) => o !== id);
		}
	}

	function valider(r: ResumeBrouillon): Promise<void> {
		const id = lireUuid(r.id);
		if (id === null) return Promise.resolve();
		return surBrouillon(r.id, async () => {
			const v = await validerBrouillon(api, cabinet, dossier, id, r.version);
			retirerDeLaListe([r.id]);
			validees = [{ id: v.id, numero: v.numero }];
			info = `Écriture validée sous le numéro ${v.numero}.`;
			avertissements = bandeauAvertissements(
				[{ numero: v.numero, avertissements: lireAvertissements(v.avertissements) }],
				regional,
				false
			);
		});
	}

	function basculerDrapeau(r: ResumeBrouillon): Promise<void> {
		const id = lireUuid(r.id);
		if (id === null) return Promise.resolve();
		return surBrouillon(r.id, async () => {
			const b = await changerDrapeau(api, cabinet, dossier, id, !r.a_revoir);
			lignes = lignes.map((l) =>
				l.id === r.id ? { ...l, a_revoir: b.a_revoir, version: b.version } : l
			);
		});
	}

	function supprimer(r: ResumeBrouillon): Promise<void> {
		const id = lireUuid(r.id);
		if (id === null) return Promise.resolve();
		aSupprimer = null;
		return surBrouillon(r.id, async () => {
			await supprimerBrouillon(api, cabinet, dossier, id, r.version);
			retirerDeLaListe([r.id]);
			info = 'Brouillon supprimé.';
		});
	}

	// --- Sélection et lot ------------------------------------------------------------------

	function basculerSelection(id: string): void {
		selection = selection.includes(id) ? selection.filter((s) => s !== id) : [...selection, id];
		phase = 'aucune';
	}

	function toutSelectionner(coche: boolean): void {
		selection = coche ? affichees.slice(0, LOT_MAX).map((l) => l.id) : [];
		phase = 'aucune';
	}

	/** Seules les écritures cochées **et affichées** partent dans le lot. */
	const selectionVisible = $derived(
		(() => {
			const visibles = new Set<string>(affichees.map((l) => l.id));
			return selection.filter((id) => visibles.has(id));
		})()
	);
	const lotTropGrand = $derived(selectionVisible.length > LOT_MAX);
	const occupation = $derived(enCoursLot || occupes.length > 0);

	function demanderLot(): void {
		erreurLot = null;
		refusLot = [];
		info = null;
		validees = [];
		phase = 'confirmation';
	}

	function annulerLot(): void {
		phase = 'aucune';
	}

	/**
	 * Valide le lot : la session doit être fraîche (le dialogue de ré-authentification de la
	 * coquille s'ouvre sinon) et la requête est rejouée une fois si la fraîcheur expire entre-temps
	 * (`executerSensible`). Un lot refusé (409 `lot_refuse`) ne valide rien.
	 */
	async function confirmerLot(): Promise<void> {
		erreurLot = null;
		refusLot = [];
		avertissements = null;
		enCoursLot = true;
		const elements: ElementLot[] = [];
		for (const id of selectionVisible) {
			const uuid = lireUuid(id);
			const l = parId.get(id);
			if (uuid !== null && l !== undefined) elements.push({ ecriture: uuid, version: l.version });
		}
		try {
			const issue = await executerSensible(() => validerParLot(api, cabinet, dossier, elements));
			if (!issue.fait) {
				info = 'Validation annulée : la ré-authentification n’a pas eu lieu.';
				phase = 'aucune';
				return;
			}
			const lot = issue.valeur;
			retirerDeLaListe(lot.ecritures.map((e) => e.id));
			validees = lot.ecritures.map((e) => ({ id: e.id, numero: e.numero }));
			const exclues =
				lot.exclues_a_revoir > 0
					? ` ${lot.exclues_a_revoir} écriture${lot.exclues_a_revoir > 1 ? 's' : ''} « à revoir » ${lot.exclues_a_revoir > 1 ? 'restent' : 'reste'} au brouillard.`
					: '';
			info = `${lot.ecritures.length} écriture${lot.ecritures.length > 1 ? 's' : ''} validée${lot.ecritures.length > 1 ? 's' : ''}.${exclues}`;
			avertissements = bandeauAvertissements(
				lot.ecritures.map((e) => ({
					numero: e.numero,
					avertissements: lireAvertissements(e.avertissements)
				})),
				regional,
				true
			);
			phase = 'aucune';
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreurLot = e;
			if (e.detail?.motif === 'lot_refuse') refusLot = [...e.detail.refus];
			phase = 'aucune';
		} finally {
			enCoursLot = false;
		}
	}

	function messageRefus(code: string): string {
		return messageMotif(code) ?? 'Refus du serveur.';
	}

	const options = $derived(
		exercices.flatMap((x) =>
			x.periodes.map((p) => ({
				id: p.id,
				libelle: `${x.code} · ${afficherDate(p.debut, regional)} → ${afficherDate(p.fin, regional)}`
			}))
		)
	);
</script>

<form class="filtres" onsubmit={filtrer} aria-label="Filtres du brouillard">
	<div class="champ">
		<label for="filtre-journal">Journal</label>
		<select id="filtre-journal" bind:value={journalChoisi}>
			<option value="">Tous</option>
			{#each journaux as j (j.id)}
				<option value={j.code}>{j.code} — {j.intitule}</option>
			{/each}
		</select>
	</div>
	<div class="champ">
		<label for="filtre-periode">Période</label>
		<select id="filtre-periode" bind:value={periodeChoisie}>
			<option value="">Toutes</option>
			{#each options as o (o.id)}
				<option value={o.id}>{o.libelle}</option>
			{/each}
		</select>
	</div>
	<div class="champ">
		<span class="etiquette">Drapeau</span>
		<label class="case">
			<input type="checkbox" bind:checked={seulementARevoir} />
			Seulement « à revoir » (parmi les écritures chargées)
		</label>
	</div>
	<p class="filtres-action">
		<button type="submit" class="bouton bouton-secondaire">Filtrer</button>
	</p>
</form>

{#if info}
	<div class="message message-succes" role="status">
		<p>{info}</p>
		{#if validees.length > 0}
			<p>
				Numéros :
				{#each validees.slice(0, 20) as v, i (v.id)}
					<a href={resolve(`ecriture/?cabinet=${cabinet}&dossier=${dossier}&ecriture=${v.id}`)}>
						{v.numero}
					</a>{i < Math.min(validees.length, 20) - 1 ? ', ' : ''}
				{/each}
				{#if validees.length > 20}… ({validees.length} au total){/if}
			</p>
		{/if}
	</div>
{/if}
{#if avertissements}
	<div class="message message-avertissement" role="status">
		<p><strong>{avertissements.titre}</strong></p>
		<ul>
			{#each avertissements.lignes as ligne, i (i)}
				<li>{ligne}</li>
			{/each}
		</ul>
		{#if avertissements.suite}<p>{avertissements.suite}</p>{/if}
	</div>
{/if}
{#if erreurListe}
	<MessageErreur erreur={erreurListe} contexte="La suite n’a pas pu être chargée." />
{/if}

{#if lignes.length === 0}
	<p>Aucun brouillon ne correspond.</p>
	{#if peutSaisir}
		<p>
			<a href={resolve(`saisie/?cabinet=${cabinet}&dossier=${dossier}`)}>Saisir une écriture</a>
		</p>
	{/if}
{:else}
	{#if peutLot}
		<div class="barre-lot" role="group" aria-label="Validation par lot">
			<span role="status" aria-live="polite">
				{selectionVisible.length} sélectionnée{selectionVisible.length > 1 ? 's' : ''} ({LOT_MAX} au plus
				par lot)
			</span>
			<button
				type="button"
				class="bouton"
				onclick={demanderLot}
				disabled={selectionVisible.length === 0 || lotTropGrand || occupation}
			>
				Valider la sélection
			</button>
		</div>
		{#if lotTropGrand}
			<div class="message message-avertissement" role="alert">
				<p>Un lot compte {LOT_MAX} écritures au plus : désélectionnez-en.</p>
			</div>
		{/if}
	{/if}

	{#if phase === 'confirmation'}
		<div
			class="message message-avertissement"
			role="alertdialog"
			aria-labelledby="titre-confirmation"
		>
			<p id="titre-confirmation">
				Valider {selectionVisible.length} écriture{selectionVisible.length > 1 ? 's' : ''} ? Une écriture
				validée est définitive (seule une contre-passation la corrige). Le lot est validé en entier ou
				refusé en entier ; les écritures « à revoir » en sont exclues par le serveur.
			</p>
			<p class="actions">
				<button type="button" class="bouton" onclick={confirmerLot} disabled={enCoursLot}>
					Confirmer la validation
				</button>
				<button type="button" class="bouton bouton-secondaire" onclick={annulerLot}>Annuler</button>
			</p>
		</div>
	{/if}
	{#if erreurLot && refusLot.length === 0}
		<MessageMetier erreur={erreurLot} contexte="La validation par lot a échoué." />
	{/if}
	{#if refusLot.length > 0}
		<div class="message message-erreur" role="alert">
			<p>Le lot est refusé : aucune écriture n’a été validée. Écritures refusées :</p>
			<ul>
				{#each refusLot as r (r.ecriture)}
					<li><strong>{libelleDe(r.ecriture)}</strong> — {messageRefus(r.code)}</li>
				{/each}
			</ul>
		</div>
	{/if}

	<div class="defilement">
		<table class="liste-brouillard">
			<caption class="sr-only">Brouillons du dossier</caption>
			<thead>
				<tr>
					{#if peutLot}
						<th scope="col">
							<input
								type="checkbox"
								aria-label={`Sélectionner toutes les écritures affichées (${LOT_MAX} au plus)`}
								checked={selectionVisible.length > 0 &&
									selectionVisible.length >= Math.min(affichees.length, LOT_MAX)}
								disabled={occupation}
								onchange={(e) => toutSelectionner(e.currentTarget.checked)}
							/>
						</th>
					{/if}
					<th scope="col">Date</th>
					<th scope="col">Journal</th>
					<th scope="col">Libellé</th>
					<th scope="col" class="num">Débit</th>
					<th scope="col" class="num">Crédit</th>
					<th scope="col">État</th>
					<th scope="col">Actions</th>
				</tr>
			</thead>
			<tbody>
				{#each affichees as r (r.id)}
					{@const e = ecart([r.debit], [r.credit])}
					{@const occupe = occupation}
					<tr class:a-revoir={r.a_revoir}>
						{#if peutLot}
							<td>
								<input
									type="checkbox"
									aria-label={`Sélectionner ${r.libelle}`}
									checked={selection.includes(r.id)}
									disabled={occupation}
									onchange={() => basculerSelection(r.id)}
								/>
							</td>
						{/if}
						<td>{afficherDate(r.date, regional)}</td>
						<td>{r.journal}</td>
						<td>{r.libelle}</td>
						<td class="num">{formaterMontant(r.debit, regional)}</td>
						<td class="num">{formaterMontant(r.credit, regional)}</td>
						<td>
							{#if r.a_revoir}<strong class="badge">À revoir</strong>{/if}
							{#if e.sens !== 'equilibre'}<span class="badge badge-ecart">Déséquilibré</span>{/if}
						</td>
						<td class="actions-ligne">
							{#if peutSaisir}
								<a
									href={resolve(`saisie/?cabinet=${cabinet}&dossier=${dossier}&brouillon=${r.id}`)}
								>
									Ouvrir
								</a>
								<button
									type="button"
									class="bouton bouton-secondaire"
									onclick={() => basculerDrapeau(r)}
									disabled={occupe}
								>
									{r.a_revoir ? 'Revue faite' : 'À revoir'}
								</button>
								<button type="button" class="bouton" onclick={() => valider(r)} disabled={occupe}>
									Valider
								</button>
								{#if aSupprimer === r.id}
									<button
										type="button"
										class="bouton bouton-danger"
										onclick={() => supprimer(r)}
										disabled={occupe}
									>
										Confirmer la suppression
									</button>
									<button
										type="button"
										class="bouton bouton-secondaire"
										onclick={() => (aSupprimer = null)}
									>
										Annuler
									</button>
								{:else}
									<button
										type="button"
										class="bouton bouton-secondaire"
										onclick={() => (aSupprimer = r.id)}
										disabled={occupe}
									>
										Supprimer
									</button>
								{/if}
							{:else}
								<a
									href={resolve(`saisie/?cabinet=${cabinet}&dossier=${dossier}&brouillon=${r.id}`)}
								>
									Consulter
								</a>
							{/if}
						</td>
					</tr>
					{#if erreurs[r.id]}
						<tr>
							<td colspan={peutLot ? 8 : 7}>
								<MessageMetier erreur={erreurs[r.id] as ErreurApi} />
							</td>
						</tr>
					{/if}
				{/each}
			</tbody>
		</table>
	</div>

	{#if affichees.length === 0}
		<p class="aide">Aucune écriture « à revoir » parmi celles qui sont chargées.</p>
	{/if}
	{#if suivant !== null}
		<p>
			<button type="button" class="bouton bouton-secondaire" onclick={chargerSuite}>
				Charger la suite
			</button>
		</p>
	{/if}
{/if}

<style>
	.filtres {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
		gap: 0 var(--espace-4);
		align-items: end;
		margin-bottom: var(--espace-3);
	}

	.filtres select {
		min-height: var(--cible-min);
		padding: var(--espace-2) var(--espace-3);
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
	}

	.etiquette {
		font-weight: 600;
	}

	.case {
		display: flex;
		align-items: center;
		gap: var(--espace-2);
		min-height: var(--cible-min);
	}

	.filtres-action {
		margin-bottom: var(--espace-4);
	}

	.barre-lot {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--espace-3);
		margin-bottom: var(--espace-3);
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-2);
	}

	.defilement {
		overflow-x: auto;
		margin-bottom: var(--espace-4);
	}

	.liste-brouillard {
		width: 100%;
		min-width: 44rem;
		border-collapse: collapse;
	}

	.liste-brouillard th,
	.liste-brouillard td {
		padding: var(--espace-2);
		text-align: left;
		vertical-align: top;
		border-bottom: 1px solid var(--couleur-separateur);
	}

	.num {
		text-align: right !important;
		font-variant-numeric: tabular-nums;
	}

	.a-revoir {
		background: var(--couleur-avertissement-fond);
	}

	.badge {
		display: inline-block;
		border: 1px solid currentColor;
		border-radius: var(--rayon);
		padding: 0 var(--espace-2);
		font-size: var(--taille-petite);
		margin-right: var(--espace-1);
	}

	.badge-ecart {
		color: var(--couleur-erreur);
	}

	.actions-ligne {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--espace-1) var(--espace-2);
	}

	.bouton-danger {
		background: var(--couleur-erreur);
		border-color: var(--couleur-erreur);
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
