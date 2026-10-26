import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { computeVersion, getQuarter } from './version.mjs';

describe('release channels', () => {
  it('builds stable versions for main', () => {
    assert.equal(computeVersion(42, 'stable', new Date(2026, 9, 1)), '2026.4.42');
  });
  it('builds beta prereleases for dev', () => {
    assert.equal(computeVersion(42, 'beta', new Date(2026, 9, 1)), '2026.4.42-beta.0');
  });
  it('validates inputs', () => {
    assert.equal(getQuarter(0), 1);
    assert.equal(getQuarter(11), 4);
    assert.throws(() => computeVersion(-1, 'stable'), /Invalid build/);
    assert.throws(() => computeVersion(1, 'other'), /Invalid channel/);
  });
});
