<!-- Journaux : écritures en entier, dans l'ordre du serveur, avec les totaux de chaque journal. -->
<script lang="ts">
	import type { Journaux } from '#lib/api/editions.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { formaterMontant } from '#lib/montants.ts';
	import '#lib/styles/editions.css';

	let { journaux }: { journaux: Journaux } = $props();
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
							<td>{j === 0 ? afficherDate(e.date) : ''}</td>
							<td>{j === 0 ? e.libelle : ''}</td>
							<td>{l.compte}</td>
							<td>{l.tiers ?? ''}</td>
							<td class="num">{formaterMontant(l.debit)}</td>
							<td class="num">{formaterMontant(l.credit)}</td>
						</tr>
					{/each}
				{/each}
			</tbody>
			<tfoot>
				<tr>
					<th scope="row" colspan="5">Totaux du journal</th>
					<td class="num">{formaterMontant(bloc.total_debit)}</td>
					<td class="num">{formaterMontant(bloc.total_credit)}</td>
				</tr>
			</tfoot>
		</table>
	</div>
{/each}
