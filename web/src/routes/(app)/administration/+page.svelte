<!--
	Administration du cabinet : membres et rôles, invitations, réinitialisations d'accès, séparation
	des tâches. L'interface n'est qu'un clavier : toute décision (qui administre, ce qui est permis,
	ré-authentification, dernier administrateur…) est celle du serveur, dont le refus est affiché.
	Une action sensible demande d'abord une ré-authentification si la session n'est plus fraîche.
-->
<script lang="ts">
	import { invalidate } from '$app/navigation';
	import { ErreurApi } from '#lib/api/erreurs.ts';
	import {
		annulerReinitialisation,
		changerActivite,
		changerRole,
		changerSeparationTaches,
		inviter,
		MOTIFS_REINITIALISATION,
		reinitialiserAcces,
		renvoyerInvitation,
		revoquerInvitation,
		type MotifReinitialisation
	} from '#lib/api/ressources.ts';
	import type { Membre, Uuid } from '#lib/api/types.ts';
	import { api } from '#lib/application.ts';
	import AdresseInvalide from '#lib/composants/AdresseInvalide.svelte';
	import MessageErreur from '#lib/composants/MessageErreur.svelte';
	import { dateHeure } from '#lib/instants.ts';
	import { libelleMotifReinitialisation, libelleRole, ROLES_PROPOSES } from '#lib/libelles.ts';
	import { assurerFraicheur } from '#lib/reauthentification.svelte.ts';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let enCours = $state(false);
	let erreur = $state<ErreurApi | null>(null);
	let avis = $state('');

	/** Rôle choisi par membre (avant « Appliquer »). */
	let roles = $state<Record<string, string>>({});
	let invitation = $state({ courriel: '', nom: '', role: 'collaborateur' });
	let reinit = $state<{
		membre: Uuid | null;
		motifCode: MotifReinitialisation;
		motif: string;
		verifie: boolean;
	}>({ membre: null, motifCode: 'appareil_perdu', motif: '', verifie: false });

	/**
	 * Exécute une action du cabinet : ré-authentification si demandée, puis l'action, puis relecture
	 * des listes. Le message de succès est annoncé aux lecteurs d'écran.
	 */
	async function agir(
		succes: string,
		travail: () => Promise<void>,
		{ sensible = true }: { sensible?: boolean } = {}
	): Promise<boolean> {
		enCours = true;
		erreur = null;
		avis = '';
		try {
			if (sensible && !(await assurerFraicheur())) return false;
			await travail();
			avis = succes;
			await invalidate('sigma:administration');
			return true;
		} catch (e) {
			if (!(e instanceof ErreurApi)) throw e;
			erreur = e;
			return false;
		} finally {
			enCours = false;
		}
	}

	const nomDuMembre = (membres: Membre[], id: Uuid): string =>
		membres.find((m) => m.id === id)?.nom ?? 'Membre inconnu';

	function appliquerRole(cabinet: Uuid, membre: Membre): void {
		const role = roles[membre.id] ?? membre.role;
		void agir('Rôle modifié. Les sessions du membre ont été fermées.', async () => {
			await changerRole(api, cabinet, membre.id, role);
			delete roles[membre.id];
		});
	}

	function basculerActivite(cabinet: Uuid, membre: Membre): void {
		void agir(membre.actif ? 'Membre suspendu.' : 'Membre rétabli.', () =>
			changerActivite(api, cabinet, membre.id, !membre.actif)
		);
	}

	function inviterMembre(evenement: SubmitEvent, cabinet: Uuid): void {
		evenement.preventDefault();
		void agir('Invitation créée. Le lien part par courriel.', async () => {
			await inviter(api, cabinet, invitation);
			invitation = { courriel: '', nom: '', role: invitation.role };
		});
	}

	function demanderReinitialisation(evenement: SubmitEvent, cabinet: Uuid): void {
		evenement.preventDefault();
		const membre = reinit.membre;
		if (membre === null) return;
		void agir(
			'Réinitialisation demandée. Elle ne prend effet qu’après le délai de contestation.',
			async () => {
				await reinitialiserAcces(api, cabinet, membre, {
					motif_code: reinit.motifCode,
					motif: reinit.motif
				});
				reinit = { membre: null, motifCode: 'appareil_perdu', motif: '', verifie: false };
			}
		);
	}
</script>

<svelte:head>
	<title>Administration — Sigma Books</title>
</svelte:head>

{#if data.contenu === null}
	<AdresseInvalide />
{:else if !data.contenu.ok}
	<h1>Administration</h1>
	<MessageErreur erreur={data.contenu.erreur} contexte="Le cabinet n’a pas pu être chargé." />
{:else}
	{@const cabinet = data.cabinet as Uuid}
	{@const nomCabinet = data.contenu.valeur.nom}
	{@const membres = data.membres?.ok ? data.membres.valeur : []}
	<h1>Administration de {nomCabinet}</h1>

	<div role="status" aria-live="polite">
		{#if avis}<div class="message message-succes"><p>{avis}</p></div>{/if}
	</div>
	{#if erreur}
		<MessageErreur {erreur} contexte="L’action a échoué." />
	{/if}

	{#if data.reinitialisations?.ok && data.reinitialisations.valeur.length > 0}
		<section aria-labelledby="titre-alertes">
			<h2 id="titre-alertes">Alertes</h2>
			<div class="message message-avertissement" role="alert">
				<p>
					{data.reinitialisations.valeur.length === 1
						? 'Une réinitialisation d’accès est en attente.'
						: `${data.reinitialisations.valeur.length} réinitialisations d’accès sont en attente.`}
					Les alertes de sécurité du cabinet (changement de droits, réinitialisation, connexion par le
					secours, exports) sont aussi envoyées par courriel aux administrateurs.
				</p>
			</div>
		</section>
	{/if}

	<section aria-labelledby="titre-membres">
		<h2 id="titre-membres">Membres</h2>
		{#if data.membres && !data.membres.ok}
			<MessageErreur
				erreur={data.membres.erreur}
				contexte="Les membres n’ont pas pu être chargés."
			/>
		{:else if membres.length === 0}
			<p>Aucun membre.</p>
		{:else}
			<ul class="liste">
				{#each membres as membre (membre.id)}
					<li class="element">
						<div class="identite">
							<strong>{membre.nom}</strong>
							<span class="aide">{membre.courriel}</span>
							<span class="aide">
								{libelleRole(membre.role)}{membre.actif ? '' : ' — suspendu'}
							</span>
						</div>
						<div class="actions">
							<label class="visuellement-cache" for={`role-${membre.id}`}>
								Rôle de {membre.nom}
							</label>
							<select id={`role-${membre.id}`} bind:value={roles[membre.id]}>
								{#each ROLES_PROPOSES as role (role)}
									<option value={role} selected={role === membre.role}>{libelleRole(role)}</option>
								{/each}
							</select>
							<button
								type="button"
								class="bouton bouton-secondaire"
								disabled={enCours || (roles[membre.id] ?? membre.role) === membre.role}
								onclick={() => appliquerRole(cabinet, membre)}
							>
								Appliquer le rôle
							</button>
							<button
								type="button"
								class="bouton bouton-secondaire"
								disabled={enCours}
								onclick={() => basculerActivite(cabinet, membre)}
							>
								{membre.actif ? 'Suspendre' : 'Rétablir'}
							</button>
							<button
								type="button"
								class="bouton bouton-secondaire"
								disabled={enCours}
								aria-expanded={reinit.membre === membre.id}
								onclick={() => (reinit.membre = reinit.membre === membre.id ? null : membre.id)}
							>
								Réinitialiser l’accès
							</button>
						</div>
						{#if reinit.membre === membre.id}
							<form class="reinit" onsubmit={(e) => demanderReinitialisation(e, cabinet)}>
								<p class="aide">
									Retire les moyens de connexion et les sessions de {membre.nom} à l’issue d’un délai
									de 24 h, pendant lequel il peut contester. Un lien d’accès à usage unique lui est ensuite
									envoyé par courriel.
								</p>
								<div class="champ">
									<label for="motif-code">Motif</label>
									<select id="motif-code" bind:value={reinit.motifCode}>
										{#each MOTIFS_REINITIALISATION as code (code)}
											<option value={code}>{libelleMotifReinitialisation(code)}</option>
										{/each}
									</select>
								</div>
								<div class="champ">
									<label for="motif-texte">Précisions (500 caractères au plus)</label>
									<input
										id="motif-texte"
										type="text"
										required
										maxlength="500"
										autocomplete="off"
										bind:value={reinit.motif}
									/>
								</div>
								<div class="case">
									<input
										id="verif-hors-bande"
										type="checkbox"
										required
										bind:checked={reinit.verifie}
									/>
									<label for="verif-hors-bande">
										J’ai vérifié l’identité de la personne par un autre canal (appel, rencontre).
									</label>
								</div>
								<button type="submit" class="bouton" disabled={enCours}>
									Demander la réinitialisation
								</button>
							</form>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<section aria-labelledby="titre-invitations">
		<h2 id="titre-invitations">Invitations</h2>
		{#if data.invitations && !data.invitations.ok}
			<MessageErreur
				erreur={data.invitations.erreur}
				contexte="Les invitations n’ont pas pu être chargées."
			/>
		{:else if data.invitations?.ok && data.invitations.valeur.length > 0}
			<ul class="liste">
				{#each data.invitations.valeur as inv (inv.id)}
					<li class="element">
						<div class="identite">
							<strong>{inv.nom}</strong>
							<span class="aide">{inv.courriel}</span>
							<span class="aide">{libelleRole(inv.role)}, le {dateHeure(inv.cree_le)}</span>
						</div>
						<div class="actions">
							<button
								type="button"
								class="bouton bouton-secondaire"
								disabled={enCours}
								onclick={() =>
									agir('Invitation renvoyée.', () => renvoyerInvitation(api, cabinet, inv.id))}
							>
								Renvoyer
							</button>
							<button
								type="button"
								class="bouton bouton-secondaire"
								disabled={enCours}
								onclick={() =>
									agir('Invitation révoquée.', () => revoquerInvitation(api, cabinet, inv.id), {
										sensible: false
									})}
							>
								Révoquer
							</button>
						</div>
					</li>
				{/each}
			</ul>
		{:else}
			<p>Aucune invitation en attente.</p>
		{/if}

		<h3>Inviter une personne</h3>
		<form onsubmit={(e) => inviterMembre(e, cabinet)}>
			<div class="champ">
				<label for="inv-nom">Nom</label>
				<input
					id="inv-nom"
					type="text"
					required
					maxlength="200"
					autocomplete="off"
					bind:value={invitation.nom}
				/>
			</div>
			<div class="champ">
				<label for="inv-courriel">Adresse de courriel</label>
				<input
					id="inv-courriel"
					type="email"
					required
					maxlength="254"
					autocomplete="off"
					bind:value={invitation.courriel}
				/>
			</div>
			<div class="champ">
				<label for="inv-role">Rôle</label>
				<select id="inv-role" bind:value={invitation.role}>
					{#each ROLES_PROPOSES as role (role)}
						<option value={role}>{libelleRole(role)}</option>
					{/each}
				</select>
			</div>
			<button type="submit" class="bouton" disabled={enCours}>Envoyer l’invitation</button>
		</form>
	</section>

	<section aria-labelledby="titre-reinit">
		<h2 id="titre-reinit">Réinitialisations en attente</h2>
		{#if data.reinitialisations && !data.reinitialisations.ok}
			<MessageErreur
				erreur={data.reinitialisations.erreur}
				contexte="Les réinitialisations n’ont pas pu être chargées."
			/>
		{:else if data.reinitialisations?.ok && data.reinitialisations.valeur.length > 0}
			<ul class="liste">
				{#each data.reinitialisations.valeur as r (r.id)}
					<li class="element">
						<div class="identite">
							<strong>{nomDuMembre(membres, r.membre)}</strong>
							<span class="aide">
								{libelleMotifReinitialisation(r.motif)}, demandée le {dateHeure(r.cree_le)}
							</span>
							<span class="aide">Lien envoyé après le {dateHeure(r.echeance)}.</span>
						</div>
						<div class="actions">
							<button
								type="button"
								class="bouton bouton-secondaire"
								disabled={enCours}
								onclick={() =>
									agir(
										'Réinitialisation annulée.',
										() => annulerReinitialisation(api, cabinet, r.id),
										{
											sensible: false
										}
									)}
							>
								Annuler
							</button>
						</div>
					</li>
				{/each}
			</ul>
		{:else}
			<p>Aucune réinitialisation en attente.</p>
		{/if}
		<p class="aide">
			Le membre concerné est prévenu par courriel et peut contester la demande pendant le délai.
		</p>
	</section>

	<section aria-labelledby="titre-separation">
		<h2 id="titre-separation">Séparation des tâches</h2>
		<p class="aide">
			Quand elle est active, personne ne valide ses propres écritures. Sa désactivation est réservée
			aux cabinets d’une seule personne : elle est tracée et alerte les administrateurs.
		</p>
		<button
			type="button"
			class="bouton bouton-secondaire"
			disabled={enCours}
			onclick={() =>
				agir('Séparation des tâches activée.', () => changerSeparationTaches(api, cabinet, true), {
					sensible: false
				})}
		>
			Activer
		</button>
		<button
			type="button"
			class="bouton bouton-secondaire"
			disabled={enCours}
			onclick={() =>
				agir('Séparation des tâches désactivée.', () =>
					changerSeparationTaches(api, cabinet, false)
				)}
		>
			Désactiver
		</button>
	</section>
{/if}

<style>
	section {
		margin-top: var(--espace-6);
	}

	.element {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: var(--espace-3);
		padding: var(--espace-3) var(--espace-4);
	}

	.identite {
		display: flex;
		flex-direction: column;
		min-width: 0;
		overflow-wrap: anywhere;
	}

	.actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--espace-2);
	}

	.reinit {
		flex-basis: 100%;
	}

	select {
		min-height: var(--cible-min);
		padding: var(--espace-2) var(--espace-3);
		border: 1px solid var(--couleur-bordure);
		border-radius: var(--rayon);
		background: var(--couleur-fond);
		color: var(--couleur-texte);
		font: inherit;
		max-width: 100%;
	}

	.case {
		display: flex;
		gap: var(--espace-2);
		align-items: flex-start;
		margin-bottom: var(--espace-4);
	}
</style>
