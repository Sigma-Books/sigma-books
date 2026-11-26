<script lang="ts">
	import { goto, invalidateAll } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import type { PackPays } from '#lib/api/pays.ts';
	import { creerDossier } from '#lib/api/ressources.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import FormulaireContexte from '#lib/composants/FormulaireContexte.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import MessageMetier from '#lib/composants/MessageMetier.svelte';
	import {
		corpsCreation,
		creationPossiblementFaite,
		diagnostiquer,
		diagnostiquerNom,
		diagnostiquerOuverture,
		formulaireVide,
		messageProbleme,
		NOM_DOSSIER_MAX,
		ouvertureParDefaut,
		ouvertureSuivantFuseau,
		type FormulaireContexte as Formulaire,
		type FormulaireOuverture,
		type Probleme
	} from '#lib/contexte-saisie.ts';
	import { dateDuJour } from '#lib/dates.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import { libelleRole, ROLES_CREATION_DOSSIER } from '#lib/libelles.ts';
	import { estFuseau, regionalUtilisateur } from '#lib/regional.ts';
	import { tick, untrack } from 'svelte';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	/** Fuseau du navigateur : celui de la date proposée tant qu'aucun fuseau n'est choisi. */
	const fuseauNavigateur = regionalUtilisateur().fuseau;

	let nom = $state('');
	let formulaire = $state<Formulaire>(formulaireVide());
	/** Date du jour dans le fuseau choisi : le serveur calcule l'année en cours dans ce fuseau. */
	const aujourdhui = $derived(
		dateDuJour(estFuseau(formulaire.fuseau) ? formulaire.fuseau : fuseauNavigateur)
	);
	let ouverture = $state<FormulaireOuverture>(ouvertureParDefaut(dateDuJour(fuseauNavigateur)));
	let proposee = dateDuJour(fuseauNavigateur);
	let problemes = $state<Probleme[]>([]);
	let creationEnCours = $state(false);
	let erreurCreation = $state<ErreurApi | null>(null);
	let creationIncertaine = $state(false);
	let formulaireDom = $state<HTMLFormElement>();
	let zoneErreur = $state<HTMLElement>();

	// L'année et la date proposées suivent le fuseau choisi tant qu'elles n'ont pas été saisies.
	$effect(() => {
		const apres = aujourdhui;
		untrack(() => {
			ouverture = ouvertureSuivantFuseau(ouverture, proposee, apres);
			proposee = apres;
		});
	});

	const messageDe = (champ: string): string[] =>
		problemes.filter((p) => p.champ === champ).map(messageProbleme);

	/** Amène le focus sur la première erreur : le bouton est en bas, le message en haut. */
	async function montrerErreur(): Promise<void> {
		await tick();
		const premier = formulaireDom?.querySelector<HTMLElement>('[aria-invalid="true"]');
		(erreurCreation !== null || creationIncertaine ? zoneErreur : premier)?.focus();
		(erreurCreation !== null || creationIncertaine ? zoneErreur : premier)?.scrollIntoView({
			block: 'center'
		});
	}

	async function creer(evenement: SubmitEvent, pack: PackPays): Promise<void> {
		evenement.preventDefault();
		const cabinet = data.cabinet;
		if (cabinet === null) return;
		erreurCreation = null;
		creationIncertaine = false;
		problemes = [
			...diagnostiquerNom(nom),
			...diagnostiquer(formulaire, pack),
			...diagnostiquerOuverture(ouverture, aujourdhui)
		];
		const corps = corpsCreation(nom, formulaire, ouverture, pack, aujourdhui);
		if (corps === null) {
			await montrerErreur();
			return;
		}
		creationEnCours = true;
		try {
			const dossier = await creerDossier(api, cabinet, corps);
			await goto(resolve(`dossier/?cabinet=${cabinet}&dossier=${dossier.id}`));
		} catch (erreur) {
			if (!(erreur instanceof ErreurApi)) throw erreur;
			if (creationPossiblementFaite(erreur)) {
				// 201 mais réponse illisible : le dossier existe peut-être, on recharge la liste
				// au lieu de proposer de le recréer.
				creationIncertaine = true;
				await invalidateAll();
			} else {
				erreurCreation = erreur;
			}
			await montrerErreur();
		} finally {
			creationEnCours = false;
		}
	}
</script>

<svelte:head>
	<title>Dossiers — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Dossiers</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Les dossiers n’ont pas pu être chargés." />
{:else}
	{@const { membre, dossiers, pack } = data.contenu.valeur}
	<h1>Dossiers de {membre.nom}</h1>
	<p class="aide">Votre rôle : {libelleRole(membre.role)}</p>

	{#if dossiers.length === 0}
		<p>Aucun dossier ne vous est accessible dans ce cabinet.</p>
	{:else}
		<ul class="liste">
			{#each dossiers as dossier (dossier.id)}
				<li>
					<a
						class="liste-lien"
						href={resolve(`dossier/?cabinet=${membre.id}&dossier=${dossier.id}`)}
					>
						<span class="nom">{dossier.nom}</span>
						<span class="detail">{dossier.pays} · {dossier.devise}</span>
					</a>
				</li>
			{/each}
		</ul>
	{/if}

	{#if ROLES_CREATION_DOSSIER.includes(membre.role)}
		<section aria-labelledby="titre-creation">
			<h2 id="titre-creation">Nouveau dossier</h2>
			{#if pack === null || !pack.ok}
				{#if pack !== null}
					<MessageErreur
						erreur={pack.erreur}
						contexte="La liste des pays n’a pas pu être chargée : la création est impossible."
					/>
				{/if}
			{:else}
				<div bind:this={zoneErreur} tabindex="-1">
					{#if erreurCreation}
						<MessageMetier erreur={erreurCreation} contexte="Le dossier n’a pas été créé." />
					{/if}
					{#if creationIncertaine}
						<div class="message message-avertissement" role="alert">
							<p>{traduire('contexte.creation_incertaine')}</p>
						</div>
					{/if}
				</div>
				<form bind:this={formulaireDom} onsubmit={(e) => creer(e, pack.valeur)} novalidate>
					<div class="champ">
						<label for="nom-dossier">Nom du dossier</label>
						<input
							id="nom-dossier"
							name="nom"
							type="text"
							required
							autocomplete="off"
							aria-describedby="aide-nom-dossier"
							aria-invalid={messageDe('nom').length > 0}
							bind:value={nom}
						/>
						<span id="aide-nom-dossier" class="aide">{NOM_DOSSIER_MAX} caractères au plus.</span>
						{#each messageDe('nom') as m (m)}<span class="erreur-champ" role="alert">
								{m}
							</span>{/each}
					</div>

					<FormulaireContexte pack={pack.valeur} bind:formulaire {problemes} />

					<fieldset class="groupe">
						<legend>Premier exercice</legend>
						<div class="choix">
							<label>
								<input type="radio" name="nature" value="reprise" bind:group={ouverture.nature} />
								Entité déjà en activité (reprise)
							</label>
							<label>
								<input type="radio" name="nature" value="creation" bind:group={ouverture.nature} />
								Entité nouvelle
							</label>
						</div>
						{#if ouverture.nature === 'reprise'}
							<div class="champ">
								<label for="ouv-annee">Année de l’exercice repris</label>
								<input
									id="ouv-annee"
									type="text"
									inputmode="numeric"
									maxlength="4"
									autocomplete="off"
									aria-invalid={messageDe('annee').length > 0}
									bind:value={ouverture.annee}
								/>
								{#each messageDe('annee') as m (m)}
									<span class="erreur-champ" role="alert">{m}</span>
								{/each}
							</div>
						{:else}
							<div class="champ">
								<label for="ouv-debut">Début d’activité</label>
								<input
									id="ouv-debut"
									type="date"
									aria-invalid={messageDe('debut_activite').length > 0}
									bind:value={ouverture.debutActivite}
								/>
								{#each messageDe('debut_activite') as m (m)}
									<span class="erreur-champ" role="alert">{m}</span>
								{/each}
							</div>
							<div class="champ">
								<label for="ouv-fin">Fin du premier exercice</label>
								<select
									id="ouv-fin"
									aria-invalid={messageDe('fin_premier_exercice').length > 0}
									bind:value={ouverture.finPremier}
								>
									<option value="annee_de_debut">31 décembre de l’année de début</option>
									<option value="annee_suivante">31 décembre de l’année suivante</option>
								</select>
								<span class="aide">
									La seconde option n’est admise que si l’activité débute au second semestre.
								</span>
								{#each messageDe('fin_premier_exercice') as m (m)}
									<span class="erreur-champ" role="alert">{m}</span>
								{/each}
							</div>
						{/if}
						<p class="aide">
							Le serveur contrôle ces dates : une reprise porte sur l’année en cours ou la
							précédente, une entité nouvelle débute de l’année précédente à l’année suivante.
						</p>
					</fieldset>

					<button type="submit" class="bouton" disabled={creationEnCours}>Créer le dossier</button>
				</form>
			{/if}
		</section>
	{/if}
{/if}

<style>
	.erreur-champ {
		color: var(--couleur-erreur);
		font-size: var(--taille-petite);
	}
</style>
