<script lang="ts">
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		exporterIntegrite,
		lireEtatIntegrite,
		listerJetons,
		telechargerTemoin,
		verifierChaine,
		type EtatIntegrite,
		type FichierIntegrite,
		type ListeJetons,
		type RapportVerification
	} from '#lib/api/integrite.ts';
	import {
		detailAnomalie,
		messageExportAnnule,
		messageAnomalie,
		messageIntegrite
	} from '#lib/api/messages-editions.ts';
	import type { Resultat } from '#lib/api/resultat.ts';
	import type { Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import NavigationDossier from '#lib/composants/NavigationDossier.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { dateHeure } from '#lib/instants.ts';
	import { DELAI_RETARD_HORODATAGE_HEURES } from '#lib/seuils.ts';
	import { executerSensible } from '#lib/reauthentification.svelte.ts';
	import { enregistrerFichier } from '#lib/telechargement.ts';
	import '#lib/styles/editions.css';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let etat = $state<Resultat<EtatIntegrite> | null>(null);
	let jetons = $state<Resultat<ListeJetons> | null>(null);
	let rapport = $state<RapportVerification | null>(null);
	let erreurVerification = $state<ErreurApi | null>(null);
	let verificationEnCours = $state(false);
	let actualisationEnCours = $state(false);
	let erreurAction = $state<ErreurApi | null>(null);
	let messageAction = $state<string | null>(null);
	let exportEnCours = $state(false);
	let exportFait = $state<FichierIntegrite | null>(null);
	let temoinEnCours = $state(false);
	/** Change avec le dossier de l'URL : une réponse d'un autre dossier est ignorée. */
	let generation = 0;
	const cleDossier = $derived(
		data.contenu?.ok
			? `${data.contenu.valeur.contexte.cabinet}/${data.contenu.valeur.contexte.dossier}`
			: ''
	);
	$effect.pre(() => {
		void cleDossier;
		generation += 1;
		rapport = null;
		erreurVerification = null;
		erreurAction = null;
		messageAction = null;
		exportFait = null;
		verificationEnCours = false;
		actualisationEnCours = false;
		exportEnCours = false;
		temoinEnCours = false;
	});

	$effect(() => {
		if (data.contenu?.ok) {
			etat = data.contenu.valeur.valeur.etat;
			jetons = data.contenu.valeur.valeur.jetons;
		}
	});

	async function tenter<T>(travail: () => Promise<T>): Promise<Resultat<T>> {
		try {
			return { ok: true, valeur: await travail() };
		} catch (erreur) {
			if (erreur instanceof ErreurApi) return { ok: false, erreur };
			throw erreur;
		}
	}

	async function actualiser(cabinet: Uuid, dossier: Uuid): Promise<void> {
		const g = generation;
		actualisationEnCours = true;
		const [e, j] = await Promise.all([
			tenter(() => lireEtatIntegrite(api, cabinet, dossier)),
			tenter(() => listerJetons(api, cabinet, dossier))
		]);
		if (g !== generation) return;
		[etat, jetons] = [e, j];
		actualisationEnCours = false;
	}

	async function verifier(cabinet: Uuid, dossier: Uuid): Promise<void> {
		const g = generation;
		verificationEnCours = true;
		rapport = null;
		erreurVerification = null;
		const lu = await tenter(() => verifierChaine(api, cabinet, dossier));
		if (g !== generation) return;
		if (lu.ok) rapport = lu.valeur;
		else erreurVerification = lu.erreur;
		verificationEnCours = false;
	}

	async function exporter(cabinet: Uuid, dossier: Uuid): Promise<void> {
		const g = generation;
		exportEnCours = true;
		erreurAction = null;
		messageAction = null;
		exportFait = null;
		try {
			const issue = await executerSensible(() => exporterIntegrite(api, cabinet, dossier));
			// Le fichier demandé est enregistré (l'export est déjà audité) ; l'écran, lui, ne
			// montre jamais le résultat d'un autre dossier.
			if (issue.fait) enregistrerFichier(issue.valeur.contenu, issue.valeur.nom);
			if (g !== generation) return;
			if (!issue.fait) messageAction = messageExportAnnule();
			else exportFait = issue.valeur;
		} catch (erreur) {
			if (!(erreur instanceof ErreurApi)) throw erreur;
			if (g === generation) erreurAction = erreur;
		} finally {
			if (g === generation) exportEnCours = false;
		}
	}

	async function temoin(cabinet: Uuid, dossier: Uuid): Promise<void> {
		const g = generation;
		temoinEnCours = true;
		erreurAction = null;
		messageAction = null;
		try {
			const fichier = await telechargerTemoin(api, cabinet, dossier);
			enregistrerFichier(fichier.contenu, fichier.nom);
		} catch (erreur) {
			if (!(erreur instanceof ErreurApi)) throw erreur;
			if (g === generation) erreurAction = erreur;
		} finally {
			if (g === generation) temoinEnCours = false;
		}
	}
</script>

<svelte:head>
	<title>Intégrité — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Intégrité</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="L’écran n’a pas pu être chargé." />
{:else}
	{@const { contexte } = data.contenu.valeur}
	<h1>Intégrité — {contexte.nomDossier.nom}</h1>
	<NavigationDossier
		cabinet={contexte.cabinet}
		dossier={contexte.dossier}
		actif="integrite"
		role={contexte.membre.role}
	/>

	<section aria-labelledby="titre-etat">
		<h2 id="titre-etat">État de la chaîne</h2>
		{#if etat === null}
			<p role="status">Chargement…</p>
		{:else if !etat.ok}
			<div
				class={etat.erreur.code === 'interdit'
					? 'message message-avertissement'
					: 'message message-erreur'}
				role={etat.erreur.code === 'interdit' ? 'status' : 'alert'}
			>
				<p>{messageIntegrite(etat.erreur)}</p>
			</div>
		{:else}
			{@const e = etat.valeur}
			{#if e.horodatage.retard}
				<div class="message message-erreur" role="alert">
					<p>
						<strong>Retard d’horodatage</strong> : des écritures attendent un jeton depuis plus de
						{DELAI_RETARD_HORODATAGE_HEURES} h{#if e.horodatage.en_attente_depuis !== null}
							(depuis le {dateHeure(e.horodatage.en_attente_depuis, contexte.regional)}){/if}. Le
						jeton quotidien n’a pas pu être obtenu : prévenez l’administrateur.
					</p>
				</div>
			{/if}
			<dl class="fiche">
				<dt>Rang de la chaîne</dt>
				<dd>{e.nombre} écriture(s) validée(s)</dd>
				<dt>Tête de chaîne</dt>
				<dd class="mono">{e.tete}</dd>
				<dt>Jetons d’horodatage</dt>
				<dd>{e.jetons}</dd>
				<dt>Dernier jeton</dt>
				<dd>
					{#if e.dernier_jeton === null}
						Aucun jeton pour l’instant.
					{:else}
						du {dateHeure(e.dernier_jeton.date_jeton, contexte.regional)}, autorité {e.dernier_jeton
							.autorite}, couvrant {e.dernier_jeton.nombre} écriture(s)
					{/if}
				</dd>
				<dt>Écritures sans jeton</dt>
				<dd>
					{e.ecritures_sans_jeton}
					{#if e.horodatage.en_attente_depuis !== null && !e.horodatage.retard}
						<span class="aide">
							(en attente depuis le {dateHeure(e.horodatage.en_attente_depuis, contexte.regional)})
						</span>
					{/if}
				</dd>
			</dl>
			<h3>Témoin externe</h3>
			<p class="aide">
				Le témoin ne contient aucune donnée comptable (dossier, tête, nombre d’écritures) :
				conservez-le hors de cette instance pour détecter un retour en arrière de la base.
			</p>
			<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
			<pre class="mono temoin" tabindex="0" aria-label="Contenu du témoin">{e.temoin.contenu}</pre>
		{/if}
		<div class="barre-actions">
			<button
				class="bouton bouton-secondaire"
				type="button"
				disabled={actualisationEnCours}
				onclick={() => void actualiser(contexte.cabinet, contexte.dossier)}
			>
				{actualisationEnCours ? 'Actualisation…' : 'Actualiser'}
			</button>
			<button
				class="bouton bouton-secondaire"
				type="button"
				disabled={temoinEnCours}
				onclick={() => void temoin(contexte.cabinet, contexte.dossier)}
			>
				Télécharger le témoin
			</button>
		</div>
	</section>

	<section aria-labelledby="titre-verification">
		<h2 id="titre-verification">Vérification depuis la base</h2>
		<p class="aide">
			Recalcule toute la chaîne à partir des données stockées et la compare à la tête enregistrée et
			aux jetons d’horodatage. L’opération peut durer ; elle ne modifie rien.
		</p>
		<p>
			<button
				class="bouton"
				type="button"
				disabled={verificationEnCours}
				onclick={() => void verifier(contexte.cabinet, contexte.dossier)}
			>
				{verificationEnCours ? 'Vérification en cours…' : 'Lancer la vérification'}
			</button>
		</p>
		<div aria-live="polite">
			{#if erreurVerification !== null}
				<div class="message message-erreur" role="alert">
					<p>{messageIntegrite(erreurVerification)}</p>
				</div>
			{:else if rapport !== null}
				{#if rapport.integre}
					<div class="message message-succes" role="status">
						<p>
							<strong>Chaîne intègre</strong> : {rapport.nombre} écriture(s) recalculées, {rapport.jetons_valides}
							jeton(s) valide(s) sur {rapport.jetons}.
						</p>
					</div>
				{:else}
					<div class="message message-erreur" role="alert">
						<p>
							<strong>Anomalies détectées</strong>
							{#if rapport.premiere_alteration !== null}
								— première altération au rang {rapport.premiere_alteration}.
							{/if}
						</p>
					</div>
				{/if}
				{#if rapport.jetons_non_verifies > 0}
					<p class="aide">
						{rapport.jetons_non_verifies} jeton(s) parmi les plus anciens : témoin concordant, signature
						non vérifiée (borne du serveur).
					</p>
				{/if}
				{#if rapport.anomalies.length > 0}
					<div class="defilement" role="region" aria-label="Anomalies" tabindex="-1">
						<table class="table-edition">
							<caption>Anomalies ({rapport.anomalies.length} listées)</caption>
							<thead>
								<tr>
									<th scope="col">Rang</th>
									<th scope="col">Pièce</th>
									<th scope="col">Anomalie</th>
								</tr>
							</thead>
							<tbody>
								{#each rapport.anomalies as a, i (i)}
									{@const detail = detailAnomalie(a.detail)}
									<tr>
										<td>{a.rang ?? '—'}</td>
										<td>{a.numero ?? '—'}</td>
										<td>
											{messageAnomalie(a.code)}
											{#if detail !== null}<span class="mono"> ({detail})</span>{/if}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
				{#if rapport.anomalies_omises > 0}
					<div class="message message-avertissement" role="status">
						<p>
							Liste tronquée : {rapport.anomalies_omises} anomalie(s) supplémentaire(s) ne sont pas listées.
							Corrigez d’abord les premières, ou vérifiez l’export avec l’outil sigma-verifier.
						</p>
					</div>
				{/if}
			{/if}
		</div>
	</section>

	<section aria-labelledby="titre-export">
		<h2 id="titre-export">Export d’intégrité</h2>
		<p class="aide">
			Fichier de la chaîne telle que stockée, avec les jetons, vérifiable hors de l’application. Il
			exige une session récemment authentifiée, il est inscrit à l’audit du dossier et alerte les
			administrateurs.
		</p>
		<p>
			<button
				class="bouton"
				type="button"
				disabled={exportEnCours}
				onclick={() => void exporter(contexte.cabinet, contexte.dossier)}
			>
				{exportEnCours ? 'Export en cours…' : 'Exporter l’intégrité'}
			</button>
		</p>
		<div aria-live="polite">
			{#if erreurAction !== null}
				<div class="message message-erreur" role="alert">
					<p>{messageIntegrite(erreurAction)}</p>
				</div>
			{/if}
			{#if messageAction !== null}
				<div class="message message-avertissement" role="status"><p>{messageAction}</p></div>
			{/if}
			{#if exportFait !== null}
				<div class="message message-succes" role="status">
					<p>Export enregistré sous {exportFait.nom}.</p>
				</div>
				{#if exportFait.tronque === null}
					<div class="message message-avertissement" role="alert">
						<p>
							<strong>Troncature inconnue</strong> : le serveur n’a pas indiqué si tous les jetons figurent
							dans l’export. Vérifiez le fichier avec l’outil sigma-verifier avant de le conserver.
						</p>
					</div>
				{:else if exportFait.tronque}
					<div class="message message-avertissement" role="alert">
						<p>
							<strong>Export tronqué</strong> : seuls les jetons les plus récents y figurent
							{#if exportFait.jetonsExportes !== null && exportFait.jetonsTotal !== null}
								({exportFait.jetonsExportes} sur {exportFait.jetonsTotal}){/if}. L’audit du dossier
							conserve le total.
						</p>
					</div>
				{/if}
			{/if}
		</div>
	</section>

	<section aria-labelledby="titre-jetons">
		<h2 id="titre-jetons">Jetons d’horodatage</h2>
		{#if jetons === null}
			<p role="status">Chargement…</p>
		{:else if !jetons.ok}
			<div
				class={jetons.erreur.code === 'interdit'
					? 'message message-avertissement'
					: 'message message-erreur'}
				role={jetons.erreur.code === 'interdit' ? 'status' : 'alert'}
			>
				<p>{messageIntegrite(jetons.erreur)}</p>
			</div>
		{:else if jetons.valeur.jetons.length === 0}
			<p>Aucun jeton d’horodatage pour ce dossier.</p>
		{:else}
			<div class="defilement" role="region" aria-label="Jetons d’horodatage" tabindex="-1">
				<table class="table-edition">
					<caption>
						{jetons.valeur.jetons.length} jeton(s) affiché(s)
						{#if jetons.valeur.total > jetons.valeur.jetons.length}
							sur {jetons.valeur.total} (les plus récents){/if}
					</caption>
					<thead>
						<tr>
							<th scope="col">Date attestée</th>
							<th scope="col">Obtenu le</th>
							<th scope="col">Autorité</th>
							<th scope="col">Déclencheur</th>
							<th scope="col" class="num">Écritures</th>
							<th scope="col">Tête horodatée</th>
						</tr>
					</thead>
					<tbody>
						{#each jetons.valeur.jetons as j (j.id)}
							<tr>
								<td>{dateHeure(j.date_jeton, contexte.regional)}</td>
								<td>{dateHeure(j.obtenu_le, contexte.regional)}</td>
								<td>{j.autorite}</td>
								<td>{j.declencheur}</td>
								<td class="num">{j.nombre}</td>
								<td class="mono">{j.tete}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</section>
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
	}

	.temoin {
		padding: var(--espace-3);
		border: 1px solid var(--couleur-separateur);
		border-radius: var(--rayon);
		background: var(--couleur-surface);
		overflow-x: auto;
		white-space: pre-wrap;
	}

	@media (max-width: 40rem) {
		.fiche {
			grid-template-columns: 1fr;
		}
	}
</style>
