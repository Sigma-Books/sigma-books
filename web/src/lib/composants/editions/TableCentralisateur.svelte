<!-- Centralisateur des journaux (AUDCIF art. 19) : mouvements et à-nouveaux, totaux du serveur. -->
<script lang="ts">
	import type { Centralisateur, Rubrique } from '#lib/api/editions.ts';
	import { afficherMontant, libelleMois } from '#lib/editions-affichage.ts';
	import type { Regional } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	let { centralisateur, regional }: { centralisateur: Centralisateur; regional: Regional } =
		$props();
</script>

{#snippet rubrique(titre: string, r: Rubrique)}
	<div class="defilement" role="region" aria-label={titre} tabindex="-1">
		<table class="table-edition">
			<caption>{titre}</caption>
			<thead>
				<tr>
					<th scope="col">Mois</th>
					<th scope="col">Journal</th>
					<th scope="col">Compte</th>
					<th scope="col" class="num">Débit</th>
					<th scope="col" class="num">Crédit</th>
				</tr>
			</thead>
			<tbody>
				{#each r.par_journal as l, i (i)}
					<tr>
						<td>{libelleMois(l, regional)}</td>
						<td>{l.journal ?? ''}</td>
						<td></td>
						<td class="num">{afficherMontant(l.debit, regional)}</td>
						<td class="num">{afficherMontant(l.credit, regional)}</td>
					</tr>
				{/each}
				{#each r.par_mois as l, i (i)}
					<tr class="sous-total">
						<th scope="row" colspan="3">Total {libelleMois(l, regional)}</th>
						<td class="num">{afficherMontant(l.debit, regional)}</td>
						<td class="num">{afficherMontant(l.credit, regional)}</td>
					</tr>
				{/each}
			</tbody>
			<tfoot>
				<tr>
					<th scope="row" colspan="3">Total</th>
					<td class="num">{afficherMontant(r.total_debit, regional)}</td>
					<td class="num">{afficherMontant(r.total_credit, regional)}</td>
				</tr>
			</tfoot>
		</table>
	</div>
	{#if r.par_compte.length > 0}
		<details class="no-print">
			<summary>Détail par compte — {titre}</summary>
			<div
				class="defilement"
				role="region"
				aria-label={`Détail par compte, ${titre}`}
				tabindex="-1"
			>
				<table class="table-edition">
					<thead>
						<tr>
							<th scope="col">Mois</th>
							<th scope="col">Journal</th>
							<th scope="col">Compte</th>
							<th scope="col" class="num">Débit</th>
							<th scope="col" class="num">Crédit</th>
						</tr>
					</thead>
					<tbody>
						{#each r.par_compte as l, i (i)}
							<tr>
								<td>{libelleMois(l, regional)}</td>
								<td>{l.journal ?? ''}</td>
								<td>{l.compte ?? ''}</td>
								<td class="num">{afficherMontant(l.debit, regional)}</td>
								<td class="num">{afficherMontant(l.credit, regional)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		</details>
	{/if}
{/snippet}

{@render rubrique('Mouvements', centralisateur.mouvements)}
{@render rubrique('À-nouveaux', centralisateur.a_nouveaux)}
