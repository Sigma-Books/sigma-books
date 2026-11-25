<!--
	Série de codes de récupération, affichée une seule fois : le serveur ne les rend plus ensuite
	(seul leur nombre restant). Ils ne vont dans aucun stockage du navigateur.
-->
<script lang="ts">
	interface Props {
		codes: string[];
		/** Appelé quand l'utilisateur confirme les avoir conservés ; la série est alors effacée. */
		termine: () => void;
	}

	let { codes, termine }: Props = $props();

	let copie = $state<'non' | 'oui' | 'echec'>('non');

	async function copier(): Promise<void> {
		try {
			await navigator.clipboard.writeText(codes.join('\n'));
			copie = 'oui';
		} catch {
			copie = 'echec';
		}
	}
</script>

<section aria-labelledby="titre-codes" class="message message-avertissement">
	<h3 id="titre-codes">Codes de récupération</h3>
	<p>
		Conservez ces {codes.length} codes en lieu sûr (gestionnaire de mots de passe, papier). Chacun ne
		sert qu’une fois. <strong>Ils ne seront plus jamais affichés.</strong>
	</p>
	<ol class="codes">
		{#each codes as code (code)}
			<li><code>{code}</code></li>
		{/each}
	</ol>
	<p>
		<button type="button" class="bouton bouton-secondaire" onclick={copier}>
			Copier les codes
		</button>
		<button type="button" class="bouton" onclick={termine}>J’ai conservé mes codes</button>
	</p>
	<p role="status" class="aide">
		{#if copie === 'oui'}Codes copiés.{:else if copie === 'echec'}Copie impossible : recopiez les
			codes à la main.{/if}
	</p>
</section>

<style>
	.codes {
		columns: 2;
		padding-left: var(--espace-5);
		font-size: var(--taille-normale);
	}

	code {
		font-family: ui-monospace, 'Cascadia Mono', Menlo, monospace;
		overflow-wrap: anywhere;
	}

	@media (max-width: 30rem) {
		.codes {
			columns: 1;
		}
	}
</style>
