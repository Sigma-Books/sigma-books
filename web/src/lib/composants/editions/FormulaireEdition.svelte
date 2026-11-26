<!--
	Choix de l'édition et de ses filtres. Les contrôles ne sont qu'un confort : le serveur valide
	toujours (400 `requete_invalide`). Champs vides : valeurs par défaut du serveur (exercice qui
	contient la période, ou dernier ; bornes de l'exercice ; mode définitif).
-->
<script lang="ts">
	import {
		FORMATS_BALANCE,
		messageFiltre,
		NATURES_EDITION,
		verifierFiltre,
		type FiltreEdition,
		type NatureEdition
	} from '#lib/api/editions.ts';
	import type { Exercice, Journal } from '#lib/api/referentiel.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { libelleEdition, libelleFormat } from '#lib/editions-affichage.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import type { Regional } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	interface Props {
		nature: NatureEdition;
		filtre: FiltreEdition;
		/** Locale du dossier : dates des exercices. */
		regional: Regional;
		exercices: Exercice[];
		journaux: Journal[];
		enCours: boolean;
		surAfficher: () => void;
	}

	let {
		nature = $bindable(),
		filtre = $bindable(),
		regional,
		exercices,
		journaux,
		enCours,
		surAfficher
	}: Props = $props();

	let tentative = $state(false);
	const problemes = $derived(verifierFiltre(filtre, nature));

	function soumettre(evenement: SubmitEvent): void {
		evenement.preventDefault();
		tentative = true;
		if (problemes.length === 0) surAfficher();
	}
</script>

<form onsubmit={soumettre} novalidate aria-label="Filtres de l’édition">
	<fieldset class="groupe">
		<legend>Édition</legend>
		<div class="choix">
			{#each NATURES_EDITION as n (n)}
				<label>
					<input type="radio" name="nature" value={n} bind:group={nature} />
					{libelleEdition(n)}
				</label>
			{/each}
		</div>
	</fieldset>

	<div class="grille-filtre">
		<div class="champ">
			<label for="f-exercice">Exercice</label>
			<select id="f-exercice" bind:value={filtre.exercice}>
				<option value="">Par défaut</option>
				{#each exercices as e (e.id)}
					<option value={e.code}>
						{e.code} ({afficherDate(e.debut, regional)} au {afficherDate(e.fin, regional)})
					</option>
				{/each}
			</select>
		</div>
		<div class="champ">
			<label for="f-du">Du (facultatif)</label>
			<input id="f-du" type="date" bind:value={filtre.du} />
		</div>
		<div class="champ">
			<label for="f-au">Au (facultatif)</label>
			<input id="f-au" type="date" bind:value={filtre.au} />
		</div>
		{#if nature !== 'journaux'}
			<div class="champ">
				<label for="f-comptes">Comptes (facultatif)</label>
				<input
					id="f-comptes"
					type="text"
					bind:value={filtre.comptes}
					placeholder={traduire('filtre.comptes_exemple')}
					aria-describedby="aide-comptes"
					autocomplete="off"
				/>
				<span class="aide" id="aide-comptes">Numéros séparés par des virgules (50 au plus).</span>
			</div>
		{/if}
		<div class="champ">
			<label for="f-journaux">Journaux (facultatif)</label>
			<input
				id="f-journaux"
				type="text"
				bind:value={filtre.journaux}
				placeholder={journaux
					.slice(0, 3)
					.map((j) => j.code)
					.join(', ')}
				aria-describedby="aide-journaux"
				autocomplete="off"
			/>
			<span class="aide" id="aide-journaux">
				Codes séparés par des virgules
				{#if journaux.length > 0}
					({journaux.map((j) => j.code).join(', ')}){/if}.
			</span>
		</div>
		{#if nature === 'balance'}
			<div class="champ">
				<label for="f-format">Format de la balance</label>
				<select id="f-format" bind:value={filtre.format}>
					{#each FORMATS_BALANCE as f (f)}
						<option value={f}>{libelleFormat(f)}</option>
					{/each}
				</select>
			</div>
			<div class="champ">
				<label for="f-regroupement">Regroupement</label>
				<select id="f-regroupement" bind:value={filtre.regroupement}>
					<option value="compte">Par compte</option>
					<option value="tronc">Par tronc (compte de base)</option>
				</select>
			</div>
		{/if}
	</div>

	<fieldset class="groupe">
		<legend>Mode</legend>
		<div class="choix">
			<label>
				<input type="radio" name="mode" value="definitif" bind:group={filtre.mode} />
				Définitif (écritures validées)
			</label>
			<label>
				<input type="radio" name="mode" value="provisoire" bind:group={filtre.mode} />
				Provisoire (avec le brouillard équilibré)
			</label>
		</div>
	</fieldset>

	{#if tentative && problemes.length > 0}
		<div class="message message-erreur" role="alert">
			<ul>
				{#each problemes as p (p)}
					<li>{messageFiltre(p)}</li>
				{/each}
			</ul>
		</div>
	{/if}

	<p>
		<button class="bouton" type="submit" disabled={enCours}>
			{enCours ? 'Calcul en cours…' : 'Afficher l’édition'}
		</button>
	</p>
</form>
