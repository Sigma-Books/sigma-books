<!--
	Nom du dossier et devise de tenue, en tête des écrans de montants : la devise est celle du
	dossier (ADR 0027), jamais une constante de l'interface.
-->
<script lang="ts">
	import '#lib/styles/editions.css';

	interface Props {
		nom: string;
		devise: string;
	}

	let { nom, devise }: Props = $props();
</script>

<p class="identite-edition"><strong>{nom}</strong> — montants en {devise}</p>
