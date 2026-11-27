<!--
	Ce que contient la version applicable à une date : taux, régimes et motifs, avec les codes et
	les libellés que l'API fournit. L'interface n'en connaît aucun.
-->
<script lang="ts">
	import type { Applicable } from '#lib/api/parametres-fiscaux.ts';
	import '#lib/styles/editions.css';

	let { applicable }: { applicable: Applicable } = $props();
</script>

{#if applicable.taux.length > 0}
	<h3>Taux</h3>
	<div class="defilement" role="region" aria-label="Taux" tabindex="-1">
		<table class="table-edition">
			<thead>
				<tr>
					<th scope="col">Rôle</th>
					<th scope="col" class="num">Taux (%)</th>
					<th scope="col">Source</th>
				</tr>
			</thead>
			<tbody>
				{#each applicable.taux as t (t.role)}
					<tr>
						<th scope="row" class="mono">{t.role}</th>
						<td class="num">{t.valeur}</td>
						<td>{t.source}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

{#if applicable.regimes.length > 0}
	<h3>Régimes</h3>
	<div class="defilement" role="region" aria-label="Régimes" tabindex="-1">
		<table class="table-edition">
			<thead>
				<tr>
					<th scope="col">Code</th>
					<th scope="col">Libellé</th>
					<th scope="col">Exonéré</th>
					<th scope="col">Ligne de taxe</th>
					<th scope="col">Taux admis (%)</th>
					<th scope="col">Exigibilités</th>
					<th scope="col">Déduction</th>
				</tr>
			</thead>
			<tbody>
				{#each applicable.regimes as r (r.code)}
					<tr>
						<th scope="row" class="mono">{r.code}</th>
						<td>{r.libelle}</td>
						<td>{r.exonere ? 'oui' : 'non'}</td>
						<td>{r.ligne_taxe ? 'oui' : 'non'}</td>
						<td>{r.taux.length === 0 ? '—' : r.taux.join(', ')}</td>
						<td class="mono">{r.exigibilites.length === 0 ? '—' : r.exigibilites.join(', ')}</td>
						<td class="mono">{r.deduction}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}

{#if applicable.motifs.length > 0}
	<h3>Motifs d’exonération</h3>
	<div class="defilement" role="region" aria-label="Motifs d’exonération" tabindex="-1">
		<table class="table-edition">
			<thead>
				<tr>
					<th scope="col">Code</th>
					<th scope="col">Régime</th>
					<th scope="col">Libellé</th>
				</tr>
			</thead>
			<tbody>
				{#each applicable.motifs as m (m.code)}
					<tr>
						<th scope="row" class="mono">{m.code}</th>
						<td class="mono">{m.regime}</td>
						<td>{m.libelle}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
