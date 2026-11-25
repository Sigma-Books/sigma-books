<!--
	Erreur de l'API pour la saisie et la validation : le motif stable du serveur traduit en
	français (ligne citée par le serveur comprise), sinon le message général. Aucune donnée de la
	requête n'est recopiée.
-->
<script lang="ts">
	import type { ErreurApi } from '#lib/api/erreurs.ts';
	import { messageMetier } from '#lib/api/messages.ts';

	interface Props {
		erreur: ErreurApi;
		contexte?: string;
	}

	let { erreur, contexte }: Props = $props();
	const texte = $derived(
		contexte === undefined ? messageMetier(erreur) : `${contexte} ${messageMetier(erreur)}`
	);
</script>

<div class="message message-erreur" role="alert">
	<p>{texte}</p>
</div>
