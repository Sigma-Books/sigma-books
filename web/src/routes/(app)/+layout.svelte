<!--
	Coquille de l'application : en-tête (marque, compte, déconnexion), échéances de la session,
	fil d'Ariane (cabinet et dossier de l'URL) et contenu.
-->
<script lang="ts">
	import { afterNavigate, invalidate } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import { seDeconnecter } from '#lib/application.ts';
	import Echeances from '#lib/composants/Echeances.svelte';
	import DialogueReauthentification from '#lib/composants/DialogueReauthentification.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import type { LayoutProps } from './$types';

	let { data, children }: LayoutProps = $props();

	const fil = $derived(page.data.fil);
	// Une session restreinte ou d'enrôlement ne voit que l'écran des méthodes de connexion ; le
	// serveur refuse de toute façon le reste (masquage seulement).
	const complete = $derived(data.session.ok && data.session.valeur.nature === 'complete');
	let erreurDeconnexion = $state<ErreurApi | null>(null);
	let deconnexionEnCours = $state(false);

	// Chaque requête prolonge la session côté serveur : les échéances sont relues après chaque
	// navigation (la première lecture est celle du chargement initial).
	afterNavigate(({ from }) => {
		if (from !== null) void invalidate('sigma:session');
	});

	async function deconnecter(): Promise<void> {
		erreurDeconnexion = null;
		deconnexionEnCours = true;
		try {
			await seDeconnecter();
		} catch (erreur) {
			if (!(erreur instanceof ErreurApi)) throw erreur;
			erreurDeconnexion = erreur;
		} finally {
			deconnexionEnCours = false;
		}
	}

	const prolonger = () => invalidate('sigma:session');
</script>

<header class="entete">
	<div class="conteneur ligne">
		<a class="marque" href={resolve('/(app)')}>Sigma Books</a>
		<nav aria-label="Compte">
			<ul>
				<li>
					<a
						href={resolve('compte/')}
						aria-current={page.route.id === '/(app)/compte' ? 'page' : undefined}
					>
						Méthodes de connexion
					</a>
				</li>
				<li>
					<a
						href={resolve('sessions/')}
						aria-current={page.route.id === '/(app)/sessions' ? 'page' : undefined}
					>
						Mes sessions
					</a>
				</li>
				{#if fil?.cabinet}
					<li>
						<a
							href={resolve(`administration/?cabinet=${fil.cabinet.id}`)}
							aria-current={page.route.id === '/(app)/administration' ? 'page' : undefined}
						>
							Administration
						</a>
					</li>
				{/if}
				<li>
					<button
						type="button"
						class="bouton bouton-secondaire"
						onclick={deconnecter}
						disabled={deconnexionEnCours}
					>
						Se déconnecter
					</button>
				</li>
			</ul>
		</nav>
	</div>
	{#if data.session.ok}
		<div class="conteneur">
			<Echeances session={data.session.valeur} {prolonger} />
		</div>
	{/if}
</header>

{#if complete}
	<nav class="conteneur fil" aria-label="Fil d’Ariane">
		<ol>
			<li><a href={resolve('/(app)')}>Cabinets</a></li>
			{#if fil?.cabinet}
				<li>
					<a href={resolve(`dossiers/?cabinet=${fil.cabinet.id}`)}>{fil.cabinet.nom}</a>
				</li>
			{/if}
			{#if fil?.cabinet && fil.dossier}
				<li>
					<a
						href={resolve(`dossier/?cabinet=${fil.cabinet.id}&dossier=${fil.dossier.id}`)}
						aria-current="page"
					>
						{fil.dossier.nom}
					</a>
				</li>
			{/if}
		</ol>
	</nav>
{/if}

<main id="contenu" class="conteneur" tabindex="-1">
	{#if erreurDeconnexion}
		<MessageErreur erreur={erreurDeconnexion} contexte="La déconnexion a échoué." />
	{/if}
	{#if !data.session.ok}
		{#if data.session.erreur.code !== 'non_authentifie'}
			<MessageErreur erreur={data.session.erreur} contexte="La session n’a pas pu être lue." />
		{/if}
	{:else if !complete && page.route.id !== '/(app)/compte'}
		<div class="message message-avertissement" role="status">
			<p>
				Cette session ne permet que l’enregistrement d’une méthode de connexion. Elle ne donne accès
				à aucun dossier.
			</p>
			<p><a href={resolve('compte/')}>Enregistrer une méthode de connexion</a></p>
		</div>
	{:else}
		{@render children()}
	{/if}
</main>

<DialogueReauthentification />

<style>
	.entete {
		border-bottom: 1px solid var(--couleur-separateur);
		background: var(--couleur-surface);
		padding-block: var(--espace-2);
	}

	.ligne {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--espace-2);
	}

	.marque {
		font-weight: 700;
		font-size: var(--taille-titre-2);
		color: var(--couleur-texte);
		text-decoration: none;
		min-height: var(--cible-min);
		display: inline-flex;
		align-items: center;
	}

	.entete ul {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--espace-2) var(--espace-4);
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.entete ul a {
		display: inline-flex;
		align-items: center;
		min-height: var(--cible-min);
	}

	.fil ol {
		display: flex;
		flex-wrap: wrap;
		list-style: none;
		margin: 0;
		padding: var(--espace-3) 0;
		gap: var(--espace-1);
	}

	.fil li + li::before {
		content: '›';
		margin-right: var(--espace-1);
		color: var(--couleur-texte-attenue);
	}

	.fil a[aria-current='page'] {
		color: var(--couleur-texte);
		font-weight: 600;
	}

	main {
		padding-bottom: var(--espace-6);
	}

	main:focus {
		outline: none;
	}
</style>
