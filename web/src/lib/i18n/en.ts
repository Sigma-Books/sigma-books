// Catalogue anglais (Cameroun) : amorce. Seuls les messages traduits figurent ici ; tout le reste
// retombe sur le français (`fr.ts`). Une traduction garde les mêmes paramètres `{nom}` que le français.

import type { Cle } from './fr.ts';

export const en: Readonly<Partial<Record<Cle, string>>> = {
	'taxe.sigle': 'VAT',
	'contre_passation.libelle_defaut': 'Reversal of {numero}',
	'contre_passation.motif_defaut': 'Reversal of {numero}'
};
