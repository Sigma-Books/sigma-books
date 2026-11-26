<!--
	Champs du contexte d'un dossier (ADR 0027), pour la création : pays, devise de
	tenue, fuseau, langue, forme juridique et identifiants légaux. Les listes et les valeurs par
	défaut viennent du pack pays de l'API ; l'interface ne connaît aucun pays ni aucune devise. Les
	contrôles de forme (`contexte-saisie.ts`) sont un confort : le serveur décide.
-->
<script lang="ts">
	import type { PackPays } from '#lib/api/pays.ts';
	import {
		choisirPays,
		etatIdentifiants,
		FORME_JURIDIQUE_MAX,
		libelleDecalage,
		libelleLangue,
		messageProbleme,
		paysDuPack,
		VALEUR_IDENTIFIANT_MAX,
		type FormulaireContexte,
		type Probleme
	} from '#lib/contexte-saisie.ts';
	import { chercher, langueCourante } from '#lib/i18n/index.ts';
	import { LANGUES } from '#lib/regional.ts';
	import '#lib/styles/editions.css';

	interface Props {
		pack: PackPays;
		formulaire: FormulaireContexte;
		problemes: readonly Probleme[];
		/** Pays inscrit au dossier (modification) ; `null` à la création. */
		paysInitial?: string | null;
	}

	let { pack, formulaire = $bindable(), problemes, paysInitial = null }: Props = $props();

	const paysChoisi = $derived(paysDuPack(pack, formulaire.pays));
	const deviseChoisie = $derived(pack.devises.find((d) => d.code === formulaire.devise));
	/** Langues proposées : celles de l'interface, plus la langue actuelle ou celle du pays si autre. */
	const langues = $derived(
		[...new Set<string>([...LANGUES, formulaire.langue, paysChoisi?.langue ?? ''])].filter(
			(l) => l !== ''
		)
	);
	const identifiants = $derived(etatIdentifiants(formulaire, pack, paysInitial));

	function messages(champ: string): string[] {
		return problemes.filter((p) => p.champ === champ).map(messageProbleme);
	}
</script>

<div class="champ">
	<label for="ctx-pays">Pays</label>
	<select
		id="ctx-pays"
		required
		aria-describedby="aide-pays"
		aria-invalid={messages('pays').length > 0}
		value={formulaire.pays}
		onchange={(e) => (formulaire = choisirPays(formulaire, pack, e.currentTarget.value))}
	>
		{#if formulaire.pays === ''}
			<option value="" disabled>Choisir un pays</option>
		{/if}
		{#each pack.pays as p (p.code)}
			<option value={p.code}>{p.nom}</option>
		{/each}
	</select>
	<span id="aide-pays" class="aide">
		La devise de tenue, le fuseau et la langue proposés suivent le pays ; vous pouvez les changer.
	</span>
	{#each messages('pays') as m (m)}<span class="erreur-champ" role="alert">{m}</span>{/each}
</div>

<div class="champ">
	<label for="ctx-devise">Devise de tenue</label>
	<select
		id="ctx-devise"
		required
		aria-describedby="aide-devise"
		aria-invalid={messages('devise').length > 0}
		bind:value={formulaire.devise}
	>
		{#if formulaire.devise === ''}
			<option value="" disabled>Choisir une devise</option>
		{/if}
		{#each pack.devises as d (d.code)}
			<option value={d.code}>{d.code} — {d.intitule}</option>
		{/each}
	</select>
	<span id="aide-devise" class="aide">
		{#if deviseChoisie}
			Échelle de la devise : {deviseChoisie.decimales}
			{deviseChoisie.decimales > 1 ? 'décimales' : 'décimale'}.
		{/if}
		Le pays et la devise de tenue ne pourront plus être changés après la première écriture validée du
		dossier.
	</span>
	{#each messages('devise') as m (m)}<span class="erreur-champ" role="alert">{m}</span>{/each}
</div>

<div class="champ">
	<label for="ctx-fuseau">Fuseau horaire</label>
	<select
		id="ctx-fuseau"
		required
		aria-describedby="aide-fuseau"
		aria-invalid={messages('fuseau').length > 0}
		bind:value={formulaire.fuseau}
	>
		{#if formulaire.fuseau === ''}
			<option value="" disabled>Choisir un fuseau</option>
		{/if}
		{#each pack.fuseaux as f (f.nom)}
			<option value={f.nom}>{f.nom} ({libelleDecalage(f.decalage_minutes)})</option>
		{/each}
	</select>
	<span id="aide-fuseau" class="aide">
		La date du jour du dossier (écritures, validation, alertes) se calcule dans ce fuseau.
	</span>
	{#each messages('fuseau') as m (m)}<span class="erreur-champ" role="alert">{m}</span>{/each}
</div>

<div class="champ">
	<label for="ctx-langue">Langue</label>
	<select
		id="ctx-langue"
		required
		aria-invalid={messages('langue').length > 0}
		bind:value={formulaire.langue}
	>
		{#if formulaire.langue === ''}
			<option value="" disabled>Choisir une langue</option>
		{/if}
		{#each langues as l (l)}
			<option value={l}>{libelleLangue(l, langueCourante())}</option>
		{/each}
	</select>
	{#each messages('langue') as m (m)}<span class="erreur-champ" role="alert">{m}</span>{/each}
</div>

<div class="champ">
	<label for="ctx-forme">Forme juridique (facultative)</label>
	<input
		id="ctx-forme"
		type="text"
		autocomplete="off"
		aria-invalid={messages('forme_juridique').length > 0}
		aria-describedby="aide-forme"
		bind:value={formulaire.formeJuridique}
	/>
	<span id="aide-forme" class="aide">{FORME_JURIDIQUE_MAX} caractères au plus.</span>
	{#each messages('forme_juridique') as m (m)}<span class="erreur-champ" role="alert">
			{m}
		</span>{/each}
</div>

{#if identifiants.affiches.length > 0 || identifiants.retires.length > 0}
	<fieldset class="groupe">
		<legend>Identifiants légaux (facultatifs)</legend>
		<p class="aide">
			Types proposés pour le pays choisi ; la valeur est libre, de {VALEUR_IDENTIFIANT_MAX} caractères
			au plus.
		</p>
		{#if identifiants.retires.length > 0}
			<div class="message message-avertissement" role="status">
				<p>
					{chercher('contexte.identifiants_retires', { types: identifiants.retires.join(', ') })}
				</p>
			</div>
		{/if}
		{#each messages('identifiants') as m (m)}<span class="erreur-champ" role="alert">
				{m}
			</span>{/each}
		{#each identifiants.affiches as t (t.code)}
			<div class="champ">
				<label for={`ctx-id-${t.code}`}>{t.libelle} ({t.code})</label>
				<input
					id={`ctx-id-${t.code}`}
					type="text"
					autocomplete="off"
					aria-invalid={messages(`identifiant:${t.code}`).length > 0}
					bind:value={formulaire.identifiants[t.code]}
				/>
				{#each messages(`identifiant:${t.code}`) as m (m)}
					<span class="erreur-champ" role="alert">{m}</span>
				{/each}
			</div>
		{/each}
	</fieldset>
{/if}

<style>
	.erreur-champ {
		color: var(--couleur-erreur, currentColor);
		font-size: var(--taille-petite);
	}
</style>
