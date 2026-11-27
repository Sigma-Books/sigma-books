<!--
	Versions de paramètres fiscaux du dossier : celles du paquet du pays, puis les versions propres.
	Tout vient de l'API ; le motif (texte libre d'un administrateur) est affiché comme du texte,
	isolé dans un `<bdi>`.
-->
<script lang="ts">
	import type { VersionFiscale } from '#lib/api/parametres-fiscaux.ts';
	import { afficherDate } from '#lib/dates.ts';
	import { libelleAuteur } from '#lib/fiscal-affichage.ts';
	import { dateHeure } from '#lib/instants.ts';
	import { traduire } from '#lib/i18n/index.ts';
	import type { Regional } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	interface Props {
		versions: readonly VersionFiscale[];
		regional: Regional;
		/** Noms des auteurs connus (identifiant → nom) : l'API ne renvoie que l'identifiant. */
		auteurs?: ReadonlyMap<string, string>;
	}

	let { versions, regional, auteurs = new Map() }: Props = $props();
</script>

<div class="defilement" role="region" aria-label="Versions de paramètres fiscaux" tabindex="-1">
	<table class="table-edition">
		<caption class="sr-only">
			Versions de paramètres fiscaux, de la plus ancienne à la plus récente
		</caption>
		<thead>
			<tr>
				<th scope="col">Portée</th>
				<th scope="col">Version</th>
				<th scope="col">Date d’effet</th>
				<th scope="col">Date de fin</th>
				<th scope="col">Motif</th>
				<th scope="col">Auteur</th>
				<th scope="col">Créée le</th>
				<th scope="col">Empreinte</th>
			</tr>
		</thead>
		<tbody>
			{#each versions as v (v.identifiant)}
				<tr>
					<td>{v.portee === 'pays' ? 'Pays' : `Dossier (n° ${v.numero ?? '?'})`}</td>
					<th scope="row" class="mono">{v.identifiant}</th>
					<td>{afficherDate(v.date_effet, regional)}</td>
					<td>
						{#if v.date_fin}
							{afficherDate(v.date_fin, regional)}
						{:else if v.portee === 'dossier'}
							<span class="badge" title={traduire('parametres.sans_fin')}>sans fin</span>
						{:else}
							—
						{/if}
					</td>
					<td>
						{#if v.motif}<bdi>{v.motif}</bdi>{:else}—{/if}
					</td>
					<td>
						{#if v.cree_par}
							<span class="mono" title={v.cree_par}>
								<bdi>{libelleAuteur(v.cree_par, auteurs)}</bdi>
							</span>
						{:else}
							—
						{/if}
					</td>
					<td>{v.cree_le ? dateHeure(v.cree_le, regional) : '—'}</td>
					<td class="mono">{v.empreinte ? v.empreinte.slice(0, 12) : '—'}</td>
				</tr>
			{/each}
		</tbody>
	</table>
</div>
{#if versions.some((v) => v.portee === 'dossier' && !v.date_fin)}
	<p class="aide">{traduire('parametres.sans_fin')}</p>
{/if}
