// Catalogue español (Guinée équatoriale) : amorce. Voir `en.ts`.

import type { Cle } from './fr.ts';

export const es: Readonly<Partial<Record<Cle, string>>> = {
	'taxe.sigle': 'IVA',
	'contre_passation.libelle_defaut': 'Contrapartida de {numero}',
	'contre_passation.motif_defaut': 'Contrapartida de {numero}'
};
