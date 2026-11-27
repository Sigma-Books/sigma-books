<!--
	Création d'une version de paramètres fiscaux propre au dossier (ADR 0030). Les rôles de taux et
	les régimes retirables viennent de la version applicable à la date d'effet, lue à l'API ; les
	contrôles de forme (`parametres-fiscaux.ts`) sont un confort : le serveur décide. Une version
	créée est définitive : elle ne se modifie ni ne se supprime.

	La création exige une session fraîche (`executerSensible`, avec reprise unique de la requête) ;
	l'avertissement de rétroactivité est donné avant (la date est déjà atteinte) et
	après (le nombre d'écritures validées concernées, calculé par le serveur).
-->
<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { onMount, tick } from 'svelte';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		creerVersion,
		lireApplicable,
		type Applicable,
		type VersionCreee,
		type VersionFiscale
	} from '#lib/api/parametres-fiscaux.ts';
	import { longueurOctets } from '#lib/api/texte.ts';
	import type { Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import MessageMetier from '#lib/composants/MessageMetier.svelte';
	import { afficherDate, dateDuJour } from '#lib/dates.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import {
		corpsVersion,
		dateEffetLimite,
		diagnostiquerVersion,
		formulaireVersionVide,
		messageProblemeVersion,
		MOTIF_MAX_OCTETS,
		pourraitEtreRetroactive,
		regimesRetirables,
		tauxChanges,
		type FormulaireVersion,
		type ProblemeVersion
	} from '#lib/parametres-fiscaux.ts';
	import { executerSensible } from '#lib/reauthentification.svelte.ts';
	import type { Regional } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	interface Props {
		cabinet: Uuid;
		dossier: Uuid;
		versions: readonly VersionFiscale[];
		regional: Regional;
	}

	let { cabinet, dossier, versions, regional }: Props = $props();

	const aujourdhui = $derived(dateDuJour(regional.fuseau));

	let formulaire = $state<FormulaireVersion>(formulaireVersionVide());
	let base = $state<Applicable | null>(null);
	let erreurBase = $state<ErreurApi | null>(null);
	let problemes = $state<ProblemeVersion[]>([]);
	let phase = $state<'saisie' | 'confirmation' | 'fait'>('saisie');
	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let info = $state<string | null>(null);
	let creee = $state<VersionCreee | null>(null);
	let formulaireDom = $state<HTMLFormElement>();
	let zoneMessage = $state<HTMLElement>();
	let monte = true;
	/** Numéro de la dernière lecture de la base : une réponse tardive d'une date périmée est ignorée. */
	let lecture = 0;

	onMount(() => () => {
		monte = false;
	});

	const etat = $derived({ aujourdhui, versions, base });
	const retirables = $derived(base === null ? [] : regimesRetirables(base));
	const octets = $derived(longueurOctets(formulaire.motif));
	const retroactive = $derived(pourraitEtreRetroactive(formulaire.dateEffet, aujourdhui));
	const messagesDe = (champ: string): string[] =>
		problemes.filter((p) => p.champ === champ).map(messageProblemeVersion);

	async function montrer(): Promise<void> {
		await tick();
		const cible =
			erreur !== null || info !== null || erreurBase !== null
				? zoneMessage
				: formulaireDom?.querySelector<HTMLElement>('[aria-invalid="true"]');
		cible?.focus();
		cible?.scrollIntoView({ block: 'center' });
	}

	/** Lit la version qui servira de base à la date d'effet (rôles de taux et régimes offerts). */
	async function lireBase(): Promise<void> {
		const date = formulaire.dateEffet;
		const numero = ++lecture;
		base = null;
		erreurBase = null;
		if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return;
		try {
			const lue = await lireApplicable(api, cabinet, dossier, date);
			if (numero !== lecture) return;
			base = lue;
			// Les saisies portaient sur la base précédente : elles repartent de zéro.
			formulaire.taux = {};
			formulaire.retires = [];
		} catch (e) {
			if (numero !== lecture) return;
			if (e instanceof ErreurApi) erreurBase = e;
			else if (!(e instanceof TypeError)) throw e;
		}
	}

	function verifier(evenement: SubmitEvent): void {
		evenement.preventDefault();
		erreur = null;
		info = null;
		problemes = diagnostiquerVersion(formulaire, etat);
		if (problemes.length === 0) phase = 'confirmation';
		else void montrer();
	}

	/**
	 * Crée la version : ré-authentification d'abord si la session n'est plus fraîche, requête
	 * rejouée une seule fois si la fraîcheur expire entre-temps (`executerSensible`).
	 */
	async function creer(): Promise<void> {
		const corps = corpsVersion(formulaire, etat);
		if (corps === null) {
			phase = 'saisie';
			return;
		}
		erreur = null;
		info = null;
		enCours = true;
		try {
			const issue = await executerSensible(() => creerVersion(api, cabinet, dossier, corps));
			if (!monte) return;
			if (!issue.fait) {
				info = 'Création annulée : la ré-authentification n’a pas eu lieu.';
				phase = 'saisie';
				await montrer();
				return;
			}
			creee = issue.valeur;
			phase = 'fait';
			formulaire = formulaireVersionVide();
			base = null;
			await invalidateAll();
			await montrer();
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			if (!monte) return;
			erreur = e;
			phase = 'saisie';
			await montrer();
		} finally {
			if (monte) enCours = false;
		}
	}

	function recommencer(): void {
		creee = null;
		phase = 'saisie';
		info = null;
	}

	function avertissement(code: string, nombre: number): string {
		return code === 'ecritures_deja_validees'
			? traduire('parametres.avertissement.ecritures_deja_validees', { nombre })
			: traduire('avertissement.inconnu');
	}
</script>

<section aria-labelledby="titre-creation-version">
	<h2 id="titre-creation-version">Créer une version propre au dossier</h2>
	<p class="aide">{traduire('parametres.immuable')}</p>

	<div bind:this={zoneMessage} tabindex="-1">
		{#if erreur}
			<MessageMetier {erreur} contexte="La version n’a pas été créée." />
		{/if}
		{#if erreurBase}
			<MessageErreur
				erreur={erreurBase}
				contexte="Les taux et régimes applicables à cette date n’ont pas pu être lus."
			/>
		{/if}
		{#if info}
			<div class="message message-avertissement" role="status"><p>{info}</p></div>
		{/if}
		{#if creee}
			<div class="message message-succes" role="status">
				<p>
					Version <strong class="mono">{creee.identifiant}</strong> créée, effective depuis le
					{afficherDate(creee.date_effet, regional)}.
				</p>
			</div>
			{#each creee.avertissements as a (a.code)}
				<div class="message message-avertissement" role="alert">
					<p>{avertissement(a.code, a.nombre)}</p>
				</div>
			{/each}
		{/if}
	</div>

	{#if phase === 'fait'}
		<button type="button" class="bouton bouton-secondaire" onclick={recommencer}>
			Créer une autre version
		</button>
	{:else if phase === 'confirmation' && base !== null}
		{@const changes = tauxChanges(formulaire, base.taux)}
		<div class="message message-avertissement" role="alert">
			<p><strong>Confirmer la création (définitive).</strong></p>
			<ul>
				<li>
					Effet à partir du {afficherDate(formulaire.dateEffet, regional)}{formulaire.dateFin
						? `, jusqu’au ${afficherDate(formulaire.dateFin, regional)}`
						: ', sans date de fin'}.
				</li>
				{#each changes as [role, valeur] (role)}
					<li>
						Taux « {role} » : {base.taux.find((t) => t.role === role)?.valeur} % → {valeur} %.
					</li>
				{/each}
				{#each formulaire.retires as code (code)}
					<li>
						Régime retiré : {base.regimes.find((r) => r.code === code)?.libelle ?? code} ({code}).
					</li>
				{/each}
				<li>Motif : <bdi>{formulaire.motif.trim()}</bdi></li>
			</ul>
			{#if retroactive}<p>{traduire('parametres.retroactive_avant')}</p>{/if}
			{#if !formulaire.dateFin}<p>{traduire('parametres.sans_fin')}</p>{/if}
		</div>
		<div class="barre-actions">
			<button type="button" class="bouton" disabled={enCours} onclick={creer}>
				Créer la version
			</button>
			<button
				type="button"
				class="bouton bouton-secondaire"
				disabled={enCours}
				onclick={() => (phase = 'saisie')}
			>
				Modifier
			</button>
		</div>
	{:else}
		<form bind:this={formulaireDom} onsubmit={verifier} novalidate>
			<div class="champ">
				<label for="pf-date-effet">Date d’effet</label>
				<input
					id="pf-date-effet"
					type="date"
					required
					max={dateEffetLimite(aujourdhui)}
					aria-describedby="aide-date-effet"
					aria-invalid={messagesDe('date_effet').length > 0}
					bind:value={formulaire.dateEffet}
					onchange={lireBase}
				/>
				<span id="aide-date-effet" class="aide">
					Au plus un an après le jour du dossier, postérieure à la dernière version propre, hors
					période clôturée. Les taux et régimes proposés ci-dessous sont ceux de la version qui
					s’applique à cette date.
				</span>
				{#each messagesDe('date_effet') as m (m)}<span class="erreur-champ" role="alert">
						{m}
					</span>{/each}
			</div>

			{#if retroactive}
				<div class="message message-avertissement" role="status">
					<p>{traduire('parametres.retroactive_avant')}</p>
				</div>
			{/if}

			<div class="champ">
				<label for="pf-date-fin">Date de fin (facultative)</label>
				<input
					id="pf-date-fin"
					type="date"
					aria-describedby="aide-date-fin"
					aria-invalid={messagesDe('date_fin').length > 0}
					bind:value={formulaire.dateFin}
				/>
				<span id="aide-date-fin" class="aide">{traduire('parametres.sans_fin')}</span>
				{#each messagesDe('date_fin') as m (m)}<span class="erreur-champ" role="alert">
						{m}
					</span>{/each}
			</div>

			{#if base !== null && base.version !== null}
				<fieldset class="groupe">
					<legend>Taux (laisser vide pour ne pas changer)</legend>
					{#each base.taux as t (t.role)}
						<div class="champ">
							<label for={`pf-taux-${t.role}`}>Taux « {t.role} » (actuellement {t.valeur} %)</label>
							<input
								id={`pf-taux-${t.role}`}
								type="text"
								inputmode="decimal"
								autocomplete="off"
								aria-invalid={messagesDe(`taux:${t.role}`).length > 0}
								bind:value={formulaire.taux[t.role]}
							/>
							{#each messagesDe(`taux:${t.role}`) as m (m)}
								<span class="erreur-champ" role="alert">{m}</span>
							{/each}
						</div>
					{/each}
				</fieldset>
				{#if retirables.length > 0}
					<fieldset class="groupe">
						<legend>Régimes à retirer de l’offre du dossier</legend>
						<p class="aide">
							Seuls les régimes ni exonérés ni à motifs sont proposés ; le serveur refuse le retrait
							d’un régime utilisé par une écriture validée de la période d’application.
						</p>
						<div class="choix">
							{#each retirables as r (r.code)}
								<label>
									<input type="checkbox" value={r.code} bind:group={formulaire.retires} />
									{r.libelle} ({r.code})
								</label>
							{/each}
						</div>
					</fieldset>
				{/if}
				{#each messagesDe('changement') as m (m)}<span class="erreur-champ" role="alert">
						{m}
					</span>{/each}
			{:else if base !== null}
				<p class="aide">{traduire('parametres.probleme.aucune_version_applicable')}</p>
			{/if}

			<div class="champ">
				<label for="pf-motif">Motif (obligatoire)</label>
				<textarea
					id="pf-motif"
					rows="3"
					aria-describedby="aide-motif"
					aria-invalid={messagesDe('motif').length > 0}
					bind:value={formulaire.motif}></textarea>
				<span id="aide-motif" class="aide">
					{octets} / {MOTIF_MAX_OCTETS} octets. Texte libre, conservé avec la version ; il n’entre pas
					dans le journal d’audit.
				</span>
				{#each messagesDe('motif') as m (m)}<span class="erreur-champ" role="alert">
						{m}
					</span>{/each}
			</div>

			{#each messagesDe('versions') as m (m)}<span class="erreur-champ" role="alert">
					{m}
				</span>{/each}
			<button type="submit" class="bouton" disabled={enCours}>Vérifier</button>
		</form>
	{/if}
</section>

<style>
	.erreur-champ {
		color: var(--couleur-erreur);
		font-size: var(--taille-petite);
	}

	textarea {
		padding: var(--espace-2) var(--espace-3);
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
		max-width: 32rem;
		font: inherit;
	}
</style>
