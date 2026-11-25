<!--
	Pièces d'un brouillon : liste des pièces jointes (retrait), dépôt d'un fichier (PDF, JPEG,
	PNG, 20 Mio au plus) avec choix en cas de doublon, pièce déjà déposée, note d'OD interne.
	La justification est exigée par le serveur à la validation (`justification_manquante`).
-->
<script lang="ts">
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		lierPiece,
		retirerPiece,
		type Brouillon,
		type RolePieceJointe
	} from '#lib/api/brouillard.ts';
	import {
		deposerPiece,
		deposerPieceInterne,
		listerPieces,
		TAILLE_MAX_PIECE,
		type Piece
	} from '#lib/api/pieces.ts';
	import { lireUuid, type Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import { dateIso } from '#lib/dates.ts';
	import MessageMetier from '../MessageMetier.svelte';

	interface Props {
		cabinet: Uuid;
		dossier: Uuid;
		brouillon: Brouillon;
		/** Le brouillon relu après chaque changement (la version avance). */
		surChangement: (brouillon: Brouillon) => void;
	}

	let { cabinet, dossier, brouillon, surChangement }: Props = $props();

	let fichier = $state<File | null>(null);
	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let message = $state<string | null>(null);
	let doublons = $state<Piece[] | null>(null);
	let disponibles = $state<Piece[] | null>(null);
	let choisie = $state('');
	let motifNote = $state('');
	let dateNote = $state(dateIso(new Date()));

	const roleParDefaut = $derived<RolePieceJointe>(
		brouillon.pieces.some((p) => p.role === 'principale') ? 'complementaire' : 'principale'
	);
	let role = $derived<RolePieceJointe>(roleParDefaut);

	async function agir(travail: () => Promise<void>): Promise<void> {
		enCours = true;
		erreur = null;
		message = null;
		try {
			await travail();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = false;
		}
	}

	async function joindre(piece: Uuid): Promise<void> {
		const b = await lierPiece(api, cabinet, dossier, brouillon.id, piece, role, brouillon.version);
		surChangement(b);
		message = 'Pièce jointe au brouillon.';
	}

	function choixFichier(evenement: Event): void {
		const entree = evenement.currentTarget;
		fichier = entree instanceof HTMLInputElement ? (entree.files?.[0] ?? null) : null;
		doublons = null;
	}

	function deposer(confirmer = false): Promise<void> {
		return agir(async () => {
			if (fichier === null) return;
			if (fichier.size > TAILLE_MAX_PIECE) {
				throw new ErreurApi('corps_trop_grand', 413);
			}
			const r = await deposerPiece(api, cabinet, dossier, fichier, confirmer);
			if (r.doublon) {
				doublons = r.pieces;
				return;
			}
			doublons = null;
			await joindre(r.piece.id);
			fichier = null;
		});
	}

	function utiliserDoublon(piece: Piece): Promise<void> {
		return agir(async () => {
			await joindre(piece.id);
			doublons = null;
			fichier = null;
		});
	}

	function chargerDisponibles(): Promise<void> {
		return agir(async () => {
			disponibles = (await listerPieces(api, cabinet, dossier)).filter((p) => !p.ecartee);
		});
	}

	function joindreChoisie(): Promise<void> {
		return agir(async () => {
			const id = lireUuid(choisie);
			if (id !== null) await joindre(id);
		});
	}

	function retirer(piece: string): Promise<void> {
		return agir(async () => {
			const id = lireUuid(piece);
			if (id === null) return;
			surChangement(await retirerPiece(api, cabinet, dossier, brouillon.id, id, brouillon.version));
			message = 'Pièce retirée du brouillon.';
		});
	}

	function creerNote(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		return agir(async () => {
			const p = await deposerPieceInterne(api, cabinet, dossier, {
				date: dateNote,
				motif: motifNote,
				lignes: []
			});
			await joindre(p.id);
			motifNote = '';
		});
	}

	const libelleRole = (r: string) => (r === 'principale' ? 'principale' : 'complémentaire');
</script>

<section class="pieces" aria-labelledby="titre-pieces">
	<h2 id="titre-pieces">Pièces justificatives</h2>

	{#if brouillon.pieces.length === 0}
		<p class="aide">Aucune pièce jointe : le serveur refusera la validation sans justification.</p>
	{:else}
		<ul class="liste-pieces">
			{#each brouillon.pieces as p (p.piece)}
				<li>
					<span>Pièce {libelleRole(p.role)} <code>{p.piece.slice(0, 8)}</code></span>
					<button
						type="button"
						class="bouton bouton-secondaire"
						onclick={() => retirer(p.piece)}
						disabled={enCours}
					>
						Retirer
					</button>
				</li>
			{/each}
		</ul>
	{/if}

	{#if erreur}
		<MessageMetier {erreur} contexte="L’opération sur les pièces a échoué." />
	{/if}
	{#if message}
		<div class="message message-succes" role="status"><p>{message}</p></div>
	{/if}

	<div class="champ">
		<label for="role-piece">Rôle de la pièce à joindre</label>
		<select id="role-piece" bind:value={role}>
			<option value="principale">Principale</option>
			<option value="complementaire">Complémentaire</option>
		</select>
	</div>

	<div class="champ">
		<label for="fichier-piece">Déposer un fichier (PDF, JPEG ou PNG, 20 Mio au plus)</label>
		<input
			id="fichier-piece"
			type="file"
			accept="application/pdf,image/jpeg,image/png"
			onchange={choixFichier}
		/>
	</div>
	<p>
		<button
			type="button"
			class="bouton"
			onclick={() => deposer(false)}
			disabled={enCours || fichier === null}
		>
			Déposer et joindre
		</button>
	</p>

	{#if doublons !== null}
		<div class="message message-avertissement" role="alert">
			<p>
				Ce fichier est déjà déposé dans le dossier ({doublons.length}
				{doublons.length > 1 ? 'pièces identiques' : 'pièce identique'}).
			</p>
			<p class="actions">
				{#each doublons.slice(0, 1) as d (d.id)}
					<button
						type="button"
						class="bouton"
						onclick={() => utiliserDoublon(d)}
						disabled={enCours}
					>
						Joindre la pièce déjà déposée
					</button>
				{/each}
				<button
					type="button"
					class="bouton bouton-secondaire"
					onclick={() => deposer(true)}
					disabled={enCours}
				>
					Déposer quand même
				</button>
			</p>
		</div>
	{/if}

	<details>
		<summary>Joindre une pièce déjà déposée dans le dossier</summary>
		{#if disponibles === null}
			<p>
				<button
					type="button"
					class="bouton bouton-secondaire"
					onclick={chargerDisponibles}
					disabled={enCours}
				>
					Afficher les pièces du dossier
				</button>
			</p>
		{:else if disponibles.length === 0}
			<p class="aide">Aucune pièce déposée dans ce dossier.</p>
		{:else}
			<div class="champ">
				<label for="piece-existante">Pièce</label>
				<select id="piece-existante" bind:value={choisie}>
					<option value="">Choisir…</option>
					{#each disponibles as p (p.id)}
						<option value={p.id}>{p.nom ?? p.id.slice(0, 8)} ({p.type})</option>
					{/each}
				</select>
			</div>
			<p>
				<button
					type="button"
					class="bouton bouton-secondaire"
					onclick={joindreChoisie}
					disabled={enCours || choisie === ''}
				>
					Joindre
				</button>
			</p>
		{/if}
	</details>

	<details>
		<summary>Rédiger une note d’opérations diverses (pièce interne)</summary>
		<form onsubmit={creerNote}>
			<div class="champ">
				<label for="date-note">Date de l’opération</label>
				<input id="date-note" type="date" bind:value={dateNote} required />
			</div>
			<div class="champ">
				<label for="motif-note">Motif</label>
				<input id="motif-note" type="text" maxlength="1000" bind:value={motifNote} required />
			</div>
			<button type="submit" class="bouton bouton-secondaire" disabled={enCours}>
				Créer la note et la joindre
			</button>
		</form>
	</details>
</section>

<style>
	.pieces {
		border-top: 1px solid var(--couleur-separateur);
		margin-top: var(--espace-5);
		padding-top: var(--espace-4);
	}

	.liste-pieces {
		list-style: none;
		padding: 0;
		margin: 0 0 var(--espace-4);
	}

	.liste-pieces li {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--espace-2);
		padding: var(--espace-1) 0;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-2);
	}

	select,
	input[type='date'],
	input[type='text'] {
		min-height: var(--cible-min);
		padding: var(--espace-2) var(--espace-3);
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
	}

	details {
		margin: var(--espace-3) 0;
	}

	summary {
		cursor: pointer;
		min-height: var(--cible-min);
		display: flex;
		align-items: center;
	}
</style>
