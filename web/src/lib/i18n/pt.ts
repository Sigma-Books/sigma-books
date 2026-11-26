// Catalogue português (Guiné-Bissau) : amorce. Voir `en.ts`.

import type { Cle } from './fr.ts';

export const pt: Readonly<Partial<Record<Cle, string>>> = {
	'taxe.sigle': 'IVA',
	'contre_passation.libelle_defaut': 'Estorno de {numero}',
	'contre_passation.motif_defaut': 'Estorno de {numero}'
};
