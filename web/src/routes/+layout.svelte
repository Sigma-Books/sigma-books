<script lang="ts">
	import '#lib/styles/global.css';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
</script>

<a class="lien-evitement" href="#contenu">Aller au contenu</a>
{@render children()}
