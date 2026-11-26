<!-- Grand livre : un bloc par compte (ouverture, mouvements, totaux, solde final du serveur). -->
<script lang="ts">
	import type { GrandLivre } from '#lib/api/editions.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { afficherMontant } from '#lib/editions-affichage.ts';
	import type { Regional } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	let { grandLivre, regional }: { grandLivre: GrandLivre; regional: Regional } = $props();
</script>

{#each grandLivre.comptes as compte (compte.compte)}
	<div class="defilement" role="region" aria-label={`Compte ${compte.compte}`} tabindex="-1">
		<table class="table-edition">
			<caption>Compte {compte.compte}</caption>
			<thead>
				<tr>
					<th scope="col">Date</th>
					<th scope="col">Journal</th>
					<th scope="col">Pièce</th>
					<th scope="col">Libellé</th>
					<th scope="col">Tiers</th>
					<th scope="col" class="num">Débit</th>
					<th scope="col" class="num">Crédit</th>
					<th scope="col" class="num">Solde</th>
				</tr>
			</thead>
			<tbody>
				<tr class="sous-total">
					<th scope="row" colspan="7">Solde d’ouverture</th>
					<td class="num">{afficherMontant(compte.solde_ouverture, regional)}</td>
				</tr>
				{#each compte.lignes as l, i (i)}
					<tr>
						<td>{afficherDate(l.date, regional)}</td>
						<td>{l.journal}</td>
						<td>{l.numero ?? 'Brouillon'}</td>
						<td>{l.libelle}</td>
						<td>{l.tiers ?? ''}</td>
						<td class="num">{afficherMontant(l.debit, regional)}</td>
						<td class="num">{afficherMontant(l.credit, regional)}</td>
						<td class="num">{afficherMontant(l.solde, regional)}</td>
					</tr>
				{/each}
			</tbody>
			<tfoot>
				<tr>
					<th scope="row" colspan="5">Totaux et solde final</th>
					<td class="num">{afficherMontant(compte.total_debit, regional)}</td>
					<td class="num">{afficherMontant(compte.total_credit, regional)}</td>
					<td class="num">{afficherMontant(compte.solde_final, regional)}</td>
				</tr>
			</tfoot>
		</table>
	</div>
{/each}
