<!--
	Erreur de l'API, présentée sans aucune donnée de la requête : le code du serveur traduit en
	français. « Introuvable » couvre aussi bien l'absent que l'inaccessible (ADR 0011).
-->
<script lang="ts">
	import { messageErreur, type ErreurApi } from '#lib/api/erreurs.ts';

	interface Props {
		erreur: ErreurApi;
		/** Ce qui a échoué, en une phrase (« Les dossiers n'ont pas pu être chargés. »). */
		contexte?: string;
	}

	let { erreur, contexte }: Props = $props();

	const texte = $derived(
		contexte === undefined ? messageErreur(erreur) : `${contexte} ${messageErreur(erreur)}`
	);
</script>

<div class="message message-erreur" role="alert">
	<p>{texte}</p>
</div>
