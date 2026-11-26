<script lang="ts">
	import { invalidate } from '$app/navigation';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { revoquerSession } from '#lib/api/ressources.ts';
	import type { SessionListee } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { dateHeure } from '#lib/instants.ts';
	import { regionalUtilisateur } from '#lib/regional.ts';
	import { libelleMethode } from '#lib/libelles.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	// Instants dans le fuseau et la langue du navigateur : ces écrans ne dépendent d'aucun dossier.
	const regional = regionalUtilisateur();

	let enCours = $state<string | null>(null);
	let erreur = $state<ErreurApi | null>(null);
	let annonce = $state('');

	async function revoquer(session: SessionListee): Promise<void> {
		enCours = session.id;
		erreur = null;
		annonce = '';
		try {
			await revoquerSession(api, session.id);
			annonce = 'Session révoquée.';
			await invalidate('sigma:sessions');
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
		} finally {
			enCours = null;
		}
	}
</script>

<svelte:head>
	<title>Mes sessions — Sigma Books</title>
</svelte:head>

<h1>Mes sessions</h1>
<p class="aide">
	Sessions ouvertes avec votre compte. Révoquez celles que vous ne reconnaissez pas : elles sont
	fermées immédiatement.
</p>

<p class="visuellement-cache" role="status">{annonce}</p>
{#if erreur}
	<MessageErreur {erreur} contexte="La session n’a pas été révoquée." />
{/if}

{#if !data.sessions.ok}
	<MessageErreur
		erreur={data.sessions.erreur}
		contexte="Les sessions n’ont pas pu être chargées."
	/>
{:else}
	<ul class="liste">
		{#each data.sessions.valeur as session (session.id)}
			<li class="session">
				<h2>
					{session.appareil ?? 'Appareil inconnu'}
					{#if session.courante}<span class="courante">(cette session)</span>{/if}
				</h2>
				<dl>
					<dt>Adresse</dt>
					<dd>{session.adresse ?? '—'}</dd>
					<dt>Connexion</dt>
					<dd>{libelleMethode(session.methode)}, le {dateHeure(session.cree_le, regional)}</dd>
					<dt>Dernière activité</dt>
					<dd>{dateHeure(session.derniere_activite, regional)}</dd>
					<dt>Échéance</dt>
					<dd>{dateHeure(session.echeance, regional)}</dd>
				</dl>
				{#if session.courante}
					<p class="aide">Pour fermer cette session, utilisez « Se déconnecter ».</p>
				{:else}
					<button
						type="button"
						class="bouton bouton-secondaire"
						onclick={() => revoquer(session)}
						disabled={enCours !== null}
						aria-describedby={`appareil-${session.id}`}
					>
						Révoquer
					</button>
					<span id={`appareil-${session.id}`} class="visuellement-cache">
						la session {session.appareil ?? 'de l’appareil inconnu'} du {dateHeure(
							session.cree_le,
							regional
						)}
					</span>
				{/if}
			</li>
		{/each}
	</ul>
{/if}

<style>
	.session {
		padding: var(--espace-3) var(--espace-4);
	}

	.session h2 {
		font-size: var(--taille-normale);
		margin-bottom: var(--espace-2);
		overflow-wrap: anywhere;
	}

	.courante {
		color: var(--couleur-succes);
		font-weight: 400;
	}

	dl {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: var(--espace-1) var(--espace-4);
		margin: 0 0 var(--espace-3);
	}

	dt {
		color: var(--couleur-texte-attenue);
	}

	dd {
		margin: 0;
		overflow-wrap: anywhere;
	}
</style>
