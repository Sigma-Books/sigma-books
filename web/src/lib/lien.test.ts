import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { jetonDuFragment } from './lien.ts';
import { grouperSecret, uriOtpauthSure } from './libelles.ts';

describe('lien', () => {
	it('jeton du fragment', () => {
		const jeton = 'A'.repeat(43);
		assert.equal(jetonDuFragment(`#${jeton}`), jeton);
		assert.equal(jetonDuFragment(''), null);
		assert.equal(jetonDuFragment('#court'), null);
		assert.equal(jetonDuFragment(`#${jeton}/../x`), null);
	});
});

describe('secours', () => {
	it('secret groupé par quatre', () => {
		assert.equal(grouperSecret('ABCDEFGHIJ'), 'ABCD EFGH IJ');
	});
	it('uri otpauth : seul le schéma otpauth://totp/ passe', () => {
		assert.equal(
			uriOtpauthSure('otpauth://totp/Sigma:a?secret=X'),
			'otpauth://totp/Sigma:a?secret=X'
		);
		assert.equal(uriOtpauthSure('javascript:alert(1)'), null);
		assert.equal(uriOtpauthSure('otpauth://totp/a b'), null);
	});
});
