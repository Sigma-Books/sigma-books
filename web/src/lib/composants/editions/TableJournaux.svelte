<!-- Journaux : écritures en entier, dans l'ordre du serveur, avec les totaux de chaque journal. -->
<script lang="ts">
	import type { Journaux } from '#lib/api/editions.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { afficherMontant } from '#lib/editions-affichage.ts';
	import type { Regional } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	let { journaux, regional }: { journaux: Journaux; regional: Regional } = $props();
</script>

{#each journaux.journaux as bloc (bloc.journal)}
	<div class="defilement" role="region" aria-label={`Journal ${bloc.journal}`} tabindex="-1">
		<table class="table-edition">
			<caption>Journal {bloc.journal}</caption>
			<thead>
				<tr>
					<th scope="col">Pièce</th>
					<th scope="col">Date</th>
					<th scope="col">Libellé</th>
					<th scope="col">Compte</th>
					<th scope="col">Tiers</th>
					<th scope="col" class="num">Débit</th>
					<th scope="col" class="num">Crédit</th>
				</tr>
			</thead>
			<tbody>
				{#each bloc.ecritures as e, i (i)}
					{#each e.lignes as l, j (j)}
						<tr>
							<td>{j === 0 ? (e.numero ?? 'Brouillon') : ''}</td>
							<td>{j === 0 ? afficherDate(e.date, regional) : ''}</td>
							<td>{j === 0 ? e.libelle : ''}</td>
							<td>{l.compte}</td>
							<td>{l.tiers ?? ''}</td>
							<td class="num">{afficherMontant(l.debit, regional)}</td>
							<td class="num">{afficherMontant(l.credit, regional)}</td>
						</tr>
					{/each}
				{/each}
			</tbody>
			<tfoot>
				<tr>
					<th scope="row" colspan="5">Totaux du journal</th>
					<td class="num">{afficherMontant(bloc.total_debit, regional)}</td>
					<td class="num">{afficherMontant(bloc.total_credit, regional)}</td>
				</tr>
			</tfoot>
		</table>
	</div>
{/each}
