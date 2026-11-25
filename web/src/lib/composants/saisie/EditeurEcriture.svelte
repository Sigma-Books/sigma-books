<!--
	Saisie d'une écriture au clavier : en-tête (journal, date, libellé) et tableau de lignes.
	Tab passe de champ en champ, Entrée à la ligne suivante, `=` propose le solde, F2 cherche un
	compte, Ctrl+Entrée enregistre au brouillard (spec §7.1).

	Aucune logique comptable : l'équilibre affiché est indicatif (calculé en décimal exact, sur des
	chaînes), le serveur contrôle comptes, tiers, équilibre et numéros et renvoie des codes stables.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		creerBrouillon,
		modifierBrouillon,
		lireBrouillon,
		type Brouillon
	} from '#lib/api/brouillard.ts';
	import { estConflitDeVersion } from '#lib/api/messages.ts';
	import type { Compte, Journal, Tiers } from '#lib/api/referentiel.ts';
	import {
		corpsEcriture,
		equilibreIndicatif,
		indexDepuisPosition,
		ligneVide,
		LIGNES_MAX,
		soldePropose,
		type EnteteSaisie,
		type LigneSaisie
	} from '#lib/api/saisie.ts';
	import type { Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import { dateIso } from '#lib/dates.ts';
	import { formaterMontant } from '#lib/montants.ts';
	import {
		actionClavier,
		entreeSansAction,
		rechercherComptes,
		type ChampSaisie
	} from '#lib/raccourcis.ts';
	import MessageMetier from '../MessageMetier.svelte';

	interface Props {
		cabinet: Uuid;
		dossier: Uuid;
		comptes: Compte[];
		journaux: Journal[];
		tiers: Tiers[];
		/** `piece` : une écriture à la fois, avec ses pièces ; `kilometre` : journal par journal. */
		mode: 'piece' | 'kilometre';
		brouillon: Brouillon | null;
		/** Appelé après chaque enregistrement réussi (création ou modification). */
		surEnregistre: (brouillon: Brouillon, creation: boolean) => void;
		/** Appelé quand l'utilisateur ouvre une écriture vide après en avoir enregistré une. */
		surNouvelle?: () => void;
	}

	let {
		cabinet,
		dossier,
		comptes,
		journaux,
		tiers,
		mode,
		brouillon,
		surEnregistre,
		surNouvelle
	}: Props = $props();

	const SEPARATEUR = ' — ';

	function enMontantSaisi(canonique: string): string {
		return canonique.replace('.', ',');
	}

	// Le parent recrée l'éditeur (`{#key}`) quand le brouillon ouvert change : seule la valeur
	// initiale compte.
	function lignesDepuis(b: Brouillon | null): LigneSaisie[] {
		if (b === null) return [ligneVide(), ligneVide()];
		return b.lignes.map((l) => ({
			compte: l.compte,
			tiers: l.tiers ?? '',
			debit: l.sens === 'debit' ? enMontantSaisi(l.montant) : '',
			credit: l.sens === 'credit' ? enMontantSaisi(l.montant) : ''
		}));
	}

	// svelte-ignore state_referenced_locally
	let entete = $state<EnteteSaisie>({
		journal: brouillon?.journal ?? '',
		date: brouillon?.date ?? dateIso(new Date()),
		libelle: brouillon?.libelle ?? '',
		dateValeur: brouillon?.date_valeur_comptable ?? ''
	});
	// svelte-ignore state_referenced_locally
	let lignes = $state<LigneSaisie[]>(lignesDepuis(brouillon));
	// svelte-ignore state_referenced_locally
	let courant = $state<Brouillon | null>(brouillon);

	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let erreurLocale = $state<string | null>(null);
	let ligneEnErreur = $state<number | null>(null);
	let message = $state<string | null>(null);
	/** Position (grille) de chaque ligne envoyée lors du dernier enregistrement. */
	let positionsEnvoyees: number[] = [];

	const tiersActifs = $derived(tiers.filter((t) => t.actif));
	const comptesSaisissables = $derived(comptes.filter((c) => c.actif && c.mouvement));
	const intituleDe = $derived(new Map(comptes.map((c) => [c.numero, c.intitule])));
	const equilibre = $derived(equilibreIndicatif(lignes));

	// Recherche de compte (F2).
	let recherche = $state<{ ligne: number; texte: string } | null>(null);
	const resultats = $derived(
		recherche === null ? [] : rechercherComptes(comptesSaisissables, recherche.texte, 8)
	);

	const idChamp = (ligne: number, champ: ChampSaisie) => `ligne-${ligne}-${champ}`;

	async function focaliser(id: string): Promise<void> {
		await tick();
		const element = document.getElementById(id);
		if (element instanceof HTMLInputElement || element instanceof HTMLSelectElement) {
			element.focus();
			if (element instanceof HTMLInputElement) element.select();
		}
	}

	function ajouterLigne(): void {
		if (lignes.length < LIGNES_MAX) lignes.push(ligneVide());
	}

	function supprimerLigne(i: number): void {
		lignes.splice(i, 1);
		if (lignes.length === 0) lignes.push(ligneVide());
		void focaliser(idChamp(Math.min(i, lignes.length - 1), 'compte'));
	}

	function champDe(cible: EventTarget | null): { ligne: number; champ: ChampSaisie } | null {
		if (!(cible instanceof HTMLElement)) return null;
		const ligne = cible.dataset.ligne;
		const champ = cible.dataset.champ as ChampSaisie | undefined;
		if (ligne === undefined || champ === undefined) return null;
		return { ligne: Number(ligne), champ };
	}

	function surTouche(evenement: KeyboardEvent): void {
		const ou = champDe(evenement.target);
		const cible = evenement.target;
		const estSaisie = cible instanceof HTMLInputElement;
		const action = actionClavier(evenement, {
			champ: ou?.champ ?? 'entete',
			vide: estSaisie ? cible.value.trim() === '' : false,
			// Un numéro de compte en cours de frappe : Entrée valide la suggestion du navigateur.
			listeOuverte:
				ou?.champ === 'compte' &&
				estSaisie &&
				cible.value.trim() !== '' &&
				!intituleDe.has(cible.value.trim())
		});
		if (action === null) {
			if (!entreeSansAction(evenement) || cible instanceof HTMLButtonElement) return;
			// Aucune soumission native : Entrée sans action ne crée jamais de brouillon.
			evenement.preventDefault();
			// Dans l'en-tête, elle mène à la première ligne.
			const dansEntete = ou === null && (estSaisie || cible instanceof HTMLSelectElement);
			if (dansEntete && !evenement.ctrlKey && !evenement.shiftKey) {
				void focaliser(idChamp(0, 'compte'));
			}
			return;
		}
		evenement.preventDefault();
		if (action === 'enregistrer') {
			void enregistrer();
		} else if (action === 'ligne-suivante' && ou !== null) {
			if (ou.ligne === lignes.length - 1) ajouterLigne();
			void focaliser(idChamp(ou.ligne + 1, 'compte'));
		} else if (action === 'solde' && ou !== null) {
			const solde = soldePropose(lignes, ou.ligne);
			const ligne = lignes[ou.ligne];
			if (solde !== null && ligne !== undefined) {
				ligne.debit = '';
				ligne.credit = '';
				ligne[solde.champ] = enMontantSaisi(solde.montant);
			}
		} else if (action === 'recherche-compte' && ou !== null) {
			recherche = { ligne: ou.ligne, texte: lignes[ou.ligne]?.compte ?? '' };
			void focaliser('recherche-compte');
		}
	}

	function choisirCompte(numero: string): void {
		if (recherche === null) return;
		const ligne = recherche.ligne;
		const cible = lignes[ligne];
		if (cible !== undefined) cible.compte = numero;
		recherche = null;
		void focaliser(idChamp(ligne, 'debit'));
	}

	function surToucheRecherche(evenement: KeyboardEvent): void {
		if (evenement.key === 'Escape' || evenement.key === 'Enter') evenement.stopPropagation();
		if (evenement.key === 'Escape' && recherche !== null) {
			evenement.preventDefault();
			const ligne = recherche.ligne;
			recherche = null;
			void focaliser(idChamp(ligne, 'compte'));
		} else if (evenement.key === 'Enter') {
			evenement.preventDefault();
			const premier = resultats[0];
			if (premier !== undefined) choisirCompte(premier.numero);
		}
	}

	const PROBLEMES = {
		montant_ambigu:
			'Montant ambigu (« 1,500 » : mille cinq cents ou un et demi ?) : écrivez 1500 ou 1,5.',
		montant_illisible: 'Montant illisible : saisissez un nombre positif (ex. 1 500,50).',
		debit_et_credit: 'Une ligne ne porte qu’un montant : débit ou crédit, pas les deux.',
		montant_absent: 'Cette ligne n’a ni débit ni crédit.',
		compte_absent: 'Cette ligne n’a pas de compte.'
	} as const;

	async function enregistrer(): Promise<void> {
		if (enCours) return;
		erreur = null;
		erreurLocale = null;
		ligneEnErreur = null;
		message = null;
		const construit = corpsEcriture(entete, lignes, courant?.version);
		if (!construit.ok) {
			erreurLocale = `Ligne ${construit.ligne + 1} : ${PROBLEMES[construit.probleme]}`;
			ligneEnErreur = construit.ligne;
			void focaliser(idChamp(construit.ligne, 'compte'));
			return;
		}
		positionsEnvoyees = construit.positions;
		enCours = true;
		try {
			const actuel = courant;
			const creation = actuel === null;
			const b =
				actuel === null
					? await creerBrouillon(api, cabinet, dossier, construit.corps)
					: await modifierBrouillon(api, cabinet, dossier, actuel.id, construit.corps);
			courant = b;
			message = creation
				? 'Écriture enregistrée au brouillard.'
				: `Brouillon mis à jour (version ${b.version}).`;
			surEnregistre(b, creation);
			if (creation && mode === 'kilometre') nouvelle();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
			const position = e.detail?.ligne ?? null;
			if (position !== null) {
				ligneEnErreur = indexDepuisPosition(positionsEnvoyees, position);
				if (ligneEnErreur !== null) void focaliser(idChamp(ligneEnErreur, 'compte'));
			}
		} finally {
			enCours = false;
		}
	}

	/** Écriture suivante : en saisie au kilomètre, journal et date sont conservés. */
	function nouvelle(): void {
		courant = null;
		surNouvelle?.();
		entete.libelle = '';
		entete.dateValeur = '';
		lignes = [ligneVide(), ligneVide()];
		void focaliser('entete-libelle');
	}

	async function recharger(): Promise<void> {
		if (courant === null) return;
		enCours = true;
		erreur = null;
		try {
			const b = await lireBrouillon(api, cabinet, dossier, courant.id);
			courant = b;
			entete = {
				journal: b.journal,
				date: b.date,
				libelle: b.libelle,
				dateValeur: b.date_valeur_comptable ?? ''
			};
			lignes = lignesDepuis(b);
			message = 'Version actuelle du brouillon rechargée.';
			surEnregistre(b, false);
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = false;
		}
	}

	/** Met à jour la version quand les pièces du brouillon changent (le parent relit le brouillon). */
	$effect(() => {
		if (brouillon !== null && courant !== null && brouillon.id === courant.id) {
			if (brouillon.version > courant.version) courant = brouillon;
		}
	});

	const etiquetteEnregistrer = $derived(
		courant === null ? 'Enregistrer au brouillard' : 'Mettre à jour le brouillon'
	);
</script>

<!-- Les touches viennent des champs du formulaire : l'écouteur ne fait que les intercepter. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
	class="editeur"
	onsubmit={(e) => {
		e.preventDefault();
		void enregistrer();
	}}
	onkeydown={surTouche}
>
	<fieldset class="entete" disabled={enCours}>
		<legend>
			Écriture {mode === 'kilometre' ? '(saisie au kilomètre)' : '(saisie par pièce)'}
		</legend>
		<div class="champ">
			<label for="entete-journal">Journal</label>
			<select id="entete-journal" bind:value={entete.journal} required>
				<option value="" disabled>Choisir…</option>
				{#each journaux as j (j.id)}
					<option value={j.code}>{j.code}{SEPARATEUR}{j.intitule}</option>
				{/each}
			</select>
		</div>
		<div class="champ">
			<label for="entete-date">Date comptable</label>
			<input id="entete-date" type="date" bind:value={entete.date} required />
		</div>
		<div class="champ champ-large">
			<label for="entete-libelle">Libellé</label>
			<input id="entete-libelle" type="text" bind:value={entete.libelle} maxlength="200" required />
		</div>
		<div class="champ">
			<label for="entete-valeur">Date de valeur (facultative)</label>
			<input id="entete-valeur" type="date" bind:value={entete.dateValeur} />
		</div>
	</fieldset>

	<p class="aide" id="aide-raccourcis">
		Raccourcis : <kbd>Tab</kbd> champ suivant, <kbd>Entrée</kbd> ligne suivante, <kbd>=</kbd> dans
		un montant vide propose le solde, <kbd>F2</kbd> dans le compte ouvre la recherche,
		<kbd>Ctrl</kbd>+<kbd>Entrée</kbd> enregistre.
	</p>

	<datalist id="liste-comptes">
		{#each comptesSaisissables as c (c.id)}
			<option value={c.numero}>{c.numero}{SEPARATEUR}{c.intitule}</option>
		{/each}
	</datalist>

	<div class="defilement">
		<table class="grille">
			<caption class="sr-only">Lignes de l’écriture</caption>
			<thead>
				<tr>
					<th scope="col">N°</th>
					<th scope="col">Compte</th>
					<th scope="col">Tiers</th>
					<th scope="col" class="num">Débit</th>
					<th scope="col" class="num">Crédit</th>
					<th scope="col"><span class="sr-only">Actions</span></th>
				</tr>
			</thead>
			<tbody>
				{#each lignes as ligne, i (i)}
					<tr class:en-erreur={ligneEnErreur === i}>
						<th scope="row">{i + 1}</th>
						<td>
							<input
								id={idChamp(i, 'compte')}
								data-ligne={i}
								data-champ="compte"
								type="text"
								list="liste-comptes"
								inputmode="numeric"
								autocomplete="off"
								aria-label={`Compte, ligne ${i + 1}`}
								aria-describedby="aide-raccourcis"
								aria-invalid={ligneEnErreur === i ? 'true' : undefined}
								bind:value={ligne.compte}
							/>
							{#if intituleDe.get(ligne.compte.trim())}
								<small class="intitule">{intituleDe.get(ligne.compte.trim())}</small>
							{/if}
						</td>
						<td>
							<select
								id={idChamp(i, 'tiers')}
								data-ligne={i}
								data-champ="tiers"
								aria-label={`Tiers, ligne ${i + 1}`}
								bind:value={ligne.tiers}
							>
								<option value="">—</option>
								{#each tiersActifs as t (t.id)}
									<option value={t.id}>{t.nom}</option>
								{/each}
							</select>
						</td>
						<td>
							<input
								id={idChamp(i, 'debit')}
								data-ligne={i}
								data-champ="debit"
								class="num"
								type="text"
								inputmode="decimal"
								autocomplete="off"
								aria-label={`Débit, ligne ${i + 1}`}
								bind:value={ligne.debit}
							/>
						</td>
						<td>
							<input
								id={idChamp(i, 'credit')}
								data-ligne={i}
								data-champ="credit"
								class="num"
								type="text"
								inputmode="decimal"
								autocomplete="off"
								aria-label={`Crédit, ligne ${i + 1}`}
								bind:value={ligne.credit}
							/>
						</td>
						<td>
							<button
								type="button"
								class="bouton bouton-secondaire"
								onclick={() => supprimerLigne(i)}
								aria-label={`Supprimer la ligne ${i + 1}`}
							>
								×
							</button>
						</td>
					</tr>
				{/each}
			</tbody>
			<tfoot>
				<tr>
					<th scope="row" colspan="3">Totaux (indicatifs)</th>
					<td class="num">{formaterMontant(equilibre.debit)}</td>
					<td class="num">{formaterMontant(equilibre.credit)}</td>
					<td></td>
				</tr>
			</tfoot>
		</table>
	</div>

	<p class="equilibre" role="status" aria-live="polite">
		{#if equilibre.sens === 'equilibre'}
			Écriture équilibrée (indicatif).
		{:else}
			Écart de {formaterMontant(equilibre.difference)} à porter au {equilibre.sens === 'debit'
				? 'débit'
				: 'crédit'} (indicatif : le serveur contrôle l’équilibre à la validation).
		{/if}
	</p>

	{#if recherche !== null}
		<div class="recherche" role="search" aria-label="Recherche de compte">
			<label for="recherche-compte">Recherche de compte (ligne {recherche.ligne + 1})</label>
			<input
				id="recherche-compte"
				type="search"
				autocomplete="off"
				bind:value={recherche.texte}
				onkeydown={surToucheRecherche}
			/>
			{#if recherche.texte.trim() !== '' && resultats.length === 0}
				<p class="aide">Aucun compte de détail actif ne correspond.</p>
			{/if}
			<ul>
				{#each resultats as c (c.id)}
					<li>
						<button
							type="button"
							class="bouton bouton-secondaire"
							onclick={() => choisirCompte(c.numero)}
						>
							{c.numero}{SEPARATEUR}{c.intitule}
						</button>
					</li>
				{/each}
			</ul>
			<p class="aide">Entrée choisit le premier résultat, Échap ferme.</p>
		</div>
	{/if}

	{#if erreurLocale}
		<div class="message message-erreur" role="alert"><p>{erreurLocale}</p></div>
	{/if}
	{#if erreur}
		<MessageMetier {erreur} contexte="L’enregistrement a échoué." />
		{#if estConflitDeVersion(erreur)}
			<p>
				<button
					type="button"
					class="bouton bouton-secondaire"
					onclick={recharger}
					disabled={enCours}
				>
					Recharger la version actuelle (ma saisie sera remplacée)
				</button>
			</p>
		{/if}
	{/if}
	{#if message}
		<div class="message message-succes" role="status"><p>{message}</p></div>
	{/if}

	<p class="actions">
		<button type="submit" class="bouton" disabled={enCours}>{etiquetteEnregistrer}</button>
		<button
			type="button"
			class="bouton bouton-secondaire"
			onclick={ajouterLigne}
			disabled={enCours}
		>
			Ajouter une ligne
		</button>
		{#if courant !== null && mode === 'piece'}
			<button type="button" class="bouton bouton-secondaire" onclick={nouvelle} disabled={enCours}>
				Nouvelle écriture
			</button>
		{/if}
	</p>
</form>

<style>
	.entete {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
		gap: 0 var(--espace-4);
		border: 1px solid var(--couleur-separateur);
		border-radius: var(--rayon);
		padding: var(--espace-3) var(--espace-4);
		margin: 0 0 var(--espace-4);
	}

	.entete .champ-large {
		grid-column: span 2;
	}

	legend {
		font-weight: 600;
		padding-inline: var(--espace-2);
	}

	select,
	.entete input,
	.grille input {
		min-height: var(--cible-min);
		padding: var(--espace-2) var(--espace-3);
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
		width: 100%;
		min-width: 0;
	}

	.defilement {
		overflow-x: auto;
		margin-bottom: var(--espace-3);
	}

	.grille {
		width: 100%;
		min-width: 40rem;
		border-collapse: collapse;
	}

	.grille th,
	.grille td {
		padding: var(--espace-1);
		text-align: left;
		vertical-align: top;
	}

	.grille th.num,
	.grille td.num,
	.grille input.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.grille tfoot th,
	.grille tfoot td {
		border-top: 2px solid var(--couleur-separateur);
		font-weight: 600;
	}

	.en-erreur {
		background: var(--couleur-erreur-fond);
	}

	.intitule {
		display: block;
		color: var(--couleur-texte-attenue);
	}

	.equilibre {
		font-weight: 600;
	}

	.recherche {
		border: 1px solid var(--couleur-separateur);
		border-radius: var(--rayon);
		padding: var(--espace-3);
		margin-bottom: var(--espace-4);
	}

	.recherche ul {
		list-style: none;
		margin: var(--espace-2) 0;
		padding: 0;
		display: grid;
		gap: var(--espace-1);
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-2);
	}

	kbd {
		border: 1px solid var(--couleur-bordure);
		border-radius: 0.25rem;
		padding: 0 0.3em;
		font-size: 0.85em;
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
	}

	@media (max-width: 40rem) {
		.entete .champ-large {
			grid-column: span 1;
		}
	}
</style>
