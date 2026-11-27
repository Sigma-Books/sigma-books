<!--
	Création d'une version de paramètres fiscaux propre au dossier (ADR 0030). Les rôles de taux et
	les régimes retirables viennent de la version applicable à la date d'effet, lue à l'API ; les
	contrôles de forme (`parametres-fiscaux.ts`) sont un confort : le serveur décide. Une version
	créée est définitive : elle ne se modifie ni ne se supprime.

	La création exige une session fraîche (`executerSensible`, avec reprise unique de la requête) ;
	l'avertissement de rétroactivité est donné avant (des écritures validées peuvent déjà exister
	dans la période) et après (leur nombre, calculé par le serveur). Le déroulement (lectures,
	vérification, requête) est dans `creation-version.ts`, testé sous `node --test`.
-->
<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { onMount, tick } from 'svelte';
	import {
		creerVersion,
		lireApplicable,
		type VersionFiscale
	} from '#lib/api/parametres-fiscaux.ts';
	import { longueurOctets } from '#lib/api/texte.ts';
	import type { Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import MessageMetier from '#lib/composants/MessageMetier.svelte';
	import { CreationVersion } from '#lib/creation-version.ts';
	import { afficherDate, dateDuJour } from '#lib/dates.ts';
	import { afficherTaux } from '#lib/fiscal-affichage.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import {
		dateEffetLimite,
		formulaireVersionVide,
		messageProblemeVersion,
		MOTIF_MAX_OCTETS,
		regimesRetirables,
		tauxSaisis,
		type FormulaireVersion
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

	// L'écran est recréé pour chaque dossier (`{#key}` de la page) : le déroulement, ses lectures et
	// ses requêtes en vol appartiennent à un seul dossier et ne survivent pas à son départ.
	let { cabinet, dossier, versions, regional }: Props = $props();

	let formulaire = $state<FormulaireVersion>(formulaireVersionVide());
	let formulaireDom = $state<HTMLFormElement>();
	let zoneMessage = $state<HTMLElement>();

	const ctrl = new CreationVersion(
		{
			lireBase: (date) => lireApplicable(api, cabinet, dossier, date),
			creer: (corps) => creerVersion(api, cabinet, dossier, corps),
			executerSensible,
			actualiser: () => invalidateAll(),
			formulaire: () => $state.snapshot(formulaire),
			reinitialiserSaisies: (tout) => {
				if (tout) formulaire = formulaireVersionVide();
				else {
					formulaire.taux = {};
					formulaire.retires = [];
				}
			},
			aujourdhui: () => dateDuJour(regional.fuseau),
			versions: () => versions,
			regional: () => regional
		},
		() => (vue = ctrl.vue)
	);
	let vue = $state(ctrl.vue);

	onMount(() => () => ctrl.detruire());

	const aujourdhui = $derived(dateDuJour(regional.fuseau));
	const retirables = $derived(vue.base === null ? [] : regimesRetirables(vue.base));
	const octets = $derived(longueurOctets(formulaire.motif));
	const messagesDe = (champ: string): string[] =>
		vue.problemes.filter((p) => p.champ === champ).map(messageProblemeVersion);

	/** Amène le focus sur le message d'erreur ou sur le premier champ refusé : le bouton est en bas. */
	async function montrer(): Promise<void> {
		await tick();
		const message =
			vue.erreur !== null ||
			vue.erreurBase !== null ||
			vue.reauthAnnulee ||
			vue.incertaine ||
			vue.creee !== null;
		const cible = message
			? zoneMessage
			: formulaireDom?.querySelector<HTMLElement>('[aria-invalid="true"]');
		cible?.focus();
		cible?.scrollIntoView({ block: 'center' });
	}

	async function verifier(evenement: SubmitEvent): Promise<void> {
		evenement.preventDefault();
		await ctrl.verifier();
		if (vue.phase === 'saisie') await montrer();
	}

	async function creer(): Promise<void> {
		await ctrl.creer();
		await montrer();
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
		{#if vue.erreur}
			<MessageMetier erreur={vue.erreur} contexte="La version n’a pas été créée." />
		{/if}
		{#if vue.erreurBase}
			<MessageErreur
				erreur={vue.erreurBase}
				contexte="Les taux et régimes applicables à cette date n’ont pas pu être lus."
			/>
		{/if}
		{#if vue.reauthAnnulee}
			<div class="message message-avertissement" role="status">
				<p>{traduire('parametres.reauth_annulee')}</p>
			</div>
		{/if}
		{#if vue.incertaine}
			<div class="message message-avertissement" role="alert">
				<p>{traduire('parametres.creation_incertaine')}</p>
			</div>
		{/if}
		{#if vue.creee}
			{@const creee = vue.creee}
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

	{#if vue.phase === 'fait'}
		<button type="button" class="bouton bouton-secondaire" onclick={() => ctrl.recommencer()}>
			Créer une autre version
		</button>
	{:else if vue.phase === 'confirmation' && vue.base !== null}
		{@const base = vue.base}
		{@const changes = tauxSaisis(formulaire, base.taux, regional)}
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
						Taux « {role} » : {afficherTaux(
							base.taux.find((t) => t.role === role)?.valeur ?? '',
							regional
						)} % → {afficherTaux(valeur, regional)} %.
					</li>
				{/each}
				{#each formulaire.retires as code (code)}
					<li>
						Régime retiré : <bdi>{base.regimes.find((r) => r.code === code)?.libelle ?? code}</bdi>
						({code}).
					</li>
				{/each}
				<li>Motif : <bdi>{formulaire.motif.trim()}</bdi></li>
			</ul>
			<p>{traduire('parametres.retroactive_avant')}</p>
			{#if !formulaire.dateFin}<p>{traduire('parametres.sans_fin')}</p>{/if}
		</div>
		<div class="barre-actions">
			<button type="button" class="bouton" disabled={vue.enCours} onclick={creer}>
				Créer la version
			</button>
			<button
				type="button"
				class="bouton bouton-secondaire"
				disabled={vue.enCours}
				onclick={() => ctrl.modifier()}
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
					onchange={() => void ctrl.choisirDate(formulaire.dateEffet)}
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

			<div class="message message-avertissement" role="status">
				<p>{traduire('parametres.retroactive_avant')}</p>
			</div>

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

			{#if vue.lectureEnCours}
				<p class="aide" role="status">Lecture des taux et régimes applicables à cette date…</p>
			{:else if vue.base !== null && vue.base.version !== null}
				<fieldset class="groupe">
					<legend>Taux (laisser vide pour ne pas changer ; séparateur décimal du dossier)</legend>
					{#each vue.base.taux as t (t.role)}
						<div class="champ">
							<label for={`pf-taux-${t.role}`}>
								Taux « {t.role} » (actuellement {afficherTaux(t.valeur, regional)} %)
							</label>
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
									<bdi>{r.libelle}</bdi> ({r.code})
								</label>
							{/each}
						</div>
					</fieldset>
				{/if}
				{#each messagesDe('changement') as m (m)}<span class="erreur-champ" role="alert">
						{m}
					</span>{/each}
			{:else if vue.base !== null}
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
			<button type="submit" class="bouton" disabled={vue.enCours}>Vérifier</button>
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
