<!-- Navigation entre les écrans de saisie d'un dossier (saisie, brouillard, écritures validées). -->
<script lang="ts">
	import { resolve } from '$app/paths';
	import type { Uuid } from '#lib/api/types.ts';

	interface Props {
		cabinet: Uuid;
		dossier: Uuid;
		actif: 'saisie' | 'brouillard' | 'ecritures';
	}

	let { cabinet, dossier, actif }: Props = $props();
</script>

<nav class="onglets" aria-label="Écrans du dossier">
	<ul>
		<li>
			<a
				href={resolve(`saisie/?cabinet=${cabinet}&dossier=${dossier}`)}
				aria-current={actif === 'saisie' ? 'page' : undefined}
			>
				Saisie
			</a>
		</li>
		<li>
			<a
				href={resolve(`brouillard/?cabinet=${cabinet}&dossier=${dossier}`)}
				aria-current={actif === 'brouillard' ? 'page' : undefined}
			>
				Brouillard
			</a>
		</li>
		<li>
			<a
				href={resolve(`ecritures/?cabinet=${cabinet}&dossier=${dossier}`)}
				aria-current={actif === 'ecritures' ? 'page' : undefined}
			>
				Écritures validées
			</a>
		</li>
	</ul>
</nav>

<style>
	.onglets ul {
		display: flex;
		flex-wrap: wrap;
		gap: var(--espace-1) var(--espace-4);
		list-style: none;
		margin: 0 0 var(--espace-4);
		padding: 0;
		border-bottom: 1px solid var(--couleur-separateur);
	}

	a {
		display: inline-flex;
		align-items: center;
		min-height: var(--cible-min);
		text-decoration: none;
		border-bottom: 3px solid transparent;
	}

	a[aria-current='page'] {
		color: var(--couleur-texte);
		font-weight: 600;
		border-bottom-color: var(--couleur-accent);
	}
</style>
