<!-- Balance (6 colonnes générale, 2/4/8 de travail) : valeurs et totaux calculés par le serveur. -->
<script lang="ts">
	import type { Balance } from '#lib/api/editions.ts';
	import { afficherMontant, libelleColonne, titreBalance } from '#lib/editions-affichage.ts';
	import '#lib/styles/editions.css';

	let { balance }: { balance: Balance } = $props();
</script>

<div class="defilement" role="region" aria-label="Balance" tabindex="-1">
	<table class="table-edition">
		<caption>{titreBalance(balance)}</caption>
		<thead>
			<tr>
				<th scope="col">{balance.regroupement === 'tronc' ? 'Tronc' : 'Compte'}</th>
				{#each balance.colonnes as nom (nom)}
					<th scope="col" class="num">{libelleColonne(nom)}</th>
				{/each}
			</tr>
		</thead>
		<tbody>
			{#each balance.lignes as ligne (ligne.cle)}
				<tr>
					<th scope="row">{ligne.cle}</th>
					{#each ligne.valeurs as valeur, i (i)}
						<td class="num">{afficherMontant(valeur)}</td>
					{/each}
				</tr>
			{/each}
		</tbody>
		<tfoot>
			<tr>
				<th scope="row">Totaux</th>
				{#each balance.totaux as total, i (i)}
					<td class="num">{afficherMontant(total)}</td>
				{/each}
			</tr>
		</tfoot>
	</table>
</div>
<p role="status">
	{#if balance.equilibree}
		<span class="badge">Équilibrée</span> Les totaux débit et crédit sont égaux (contrôle du serveur).
	{:else}
		<span class="badge message-erreur">NON équilibrée</span> Les totaux débit et crédit diffèrent : le
		serveur signale un déséquilibre, à investiguer avant toute utilisation.
	{/if}
</p>
