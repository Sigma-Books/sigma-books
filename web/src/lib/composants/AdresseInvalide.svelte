<!-- Adresse de l'interface dont le cabinet ou le dossier n'est pas un identifiant valide. -->
<script lang="ts">
	import { resolve } from '$app/paths';
</script>

<h1>Adresse invalide</h1>
<div class="message message-erreur" role="alert">
	<p>Cette adresse ne désigne aucun cabinet ni dossier.</p>
</div>
<p><a href={resolve('/(app)')}>Choisir un cabinet</a></p>
