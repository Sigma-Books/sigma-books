<!--
	Échéances de la session (ADR 0012) : fin par inactivité et fin absolue. Cinq minutes avant la
	fin par inactivité, un avertissement propose de prolonger la session (WCAG 2.1, critère 2.2.1) :
	une simple relecture de la session suffit à noter l'activité côté serveur.
-->
<script lang="ts">
	import type { EtatSession } from '#lib/api/types.ts';
	import { dateHeure, heure, minutesRestantes } from '#lib/instants.ts';
	import { regionalUtilisateur } from '#lib/regional.ts';

	interface Props {
		session: EtatSession;
		prolonger: () => Promise<void>;
	}

	let { session, prolonger }: Props = $props();

	// Instants dans le fuseau et la langue du navigateur : ces écrans ne dépendent d'aucun dossier.
	const regional = regionalUtilisateur();

	const SEUIL_AVERTISSEMENT = 5;

	let maintenant = $state(Date.now());
	let enCours = $state(false);

	$effect(() => {
		const minuterie = setInterval(() => {
			maintenant = Date.now();
		}, 15_000);
		return () => clearInterval(minuterie);
	});

	const restant = $derived(minutesRestantes(session.fin_inactivite, maintenant));
	const bientot = $derived(restant !== null && restant <= SEUIL_AVERTISSEMENT);

	async function surProlonger(): Promise<void> {
		enCours = true;
		try {
			await prolonger();
			maintenant = Date.now();
		} finally {
			enCours = false;
		}
	}
</script>

<p class="echeances">
	Session : fin par inactivité à <time datetime={session.fin_inactivite}>
		{heure(session.fin_inactivite, regional)}
	</time>, au plus tard à
	<time datetime={session.fin_absolue}>{dateHeure(session.fin_absolue, regional)}</time>.
</p>

{#if bientot}
	<div class="message message-avertissement" role="alert">
		<p>
			{restant === 0
				? 'Votre session va se fermer pour inactivité.'
				: `Votre session se fermera pour inactivité dans ${restant} min.`}
		</p>
		<button type="button" class="bouton" onclick={surProlonger} disabled={enCours}>
			Prolonger la session
		</button>
	</div>
{/if}

<style>
	.echeances {
		margin: 0;
		color: var(--couleur-texte-attenue);
		font-size: var(--taille-petite);
	}
</style>
