'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const { buildSdkAuthoringMetadata } = require('../scripts/buildSdkAuthoringMetadata.cjs');
const { readApiPolicyMetadata } = require(path.join(process.env.KDF_SCRIPT_ANALYSIS_DIST || path.resolve(__dirname, '../dist'), 'scriptAnalysis'));

test('metadata builder preserves unknown policy rather than deriving facts from names or descriptions', () => {
  const input = { schemaVersion: 1, sdkVersion: '1', symbols: [{ symbolId: 'kdf.read.always.server', kind: 'context',
    description: 'server only; default grant; unlimited' }] };
  const result = buildSdkAuthoringMetadata(JSON.stringify(input));
  assert.equal(result.catalogVersion, null);
  assert.equal(result.defaultGrants.status, 'unknown');
  for (const key of ['binding', 'capabilities', 'authority', 'lifecycle', 'quotas', 'references', 'signature', 'deterministicInitialization'])
    assert.equal(result.symbols[0][key].status, 'unknown');
  assert.deepEqual(result.replacements, []);
  assert.deepEqual(readApiPolicyMetadata(JSON.stringify(result)), result);
});

test('metadata boundaries reject getters and executable objects without invoking them', () => {
  let executed = false;
  const hostile = { get schemaVersion() { executed = true; throw new Error('getter executed'); },
    toString() { executed = true; throw new Error('toString executed'); } };
  assert.throws(() => buildSdkAuthoringMetadata(hostile), /JSON text/);
  assert.throws(() => readApiPolicyMetadata(hostile), /JSON text/);
  assert.equal(executed, false);
});

test('metadata facts require provenance; numeric authority and exact quotas are preserved', () => {
  const provenance = { source: 'catalog-export.json', version: 'digest-1', authority: 'authoritative' };
  const input = { schemaVersion: 1, sdkVersion: '1', catalogVersion: 'digest-1', symbols: [{ symbolId: 'member', kind: 'reflected',
    authority: { value: 0, provenance }, lifecycle: { value: 2, provenance },
    quotas: { value: [{ name: 'calls', value: 10, unit: 'per tick' }], provenance } }] };
  const result = readApiPolicyMetadata(JSON.stringify(buildSdkAuthoringMetadata(JSON.stringify(input))));
  assert.equal(result.symbols[0].authority.value, 0);
  assert.equal(result.symbols[0].lifecycle.value, 2);
  assert.deepEqual(result.symbols[0].quotas.value, [{ name: 'calls', value: 10, unit: 'per tick' }]);
  input.symbols[0].capabilities = { value: [] };
  assert.throws(() => buildSdkAuthoringMetadata(JSON.stringify(input)), /provenance/);
});

test('unsupported schema, duplicates, malformed output and version-mismatched replacements reject', () => {
  const input = { schemaVersion: 1, sdkVersion: '1', catalogVersion: 'c1', symbols: [{ symbolId: 'member', kind: 'context' }] };
  assert.throws(() => buildSdkAuthoringMetadata(JSON.stringify({ ...input, schemaVersion: 2 })), /header/);
  assert.throws(() => buildSdkAuthoringMetadata(JSON.stringify({ ...input, symbols: [...input.symbols, ...input.symbols] })), /duplicate/);
  const provenance = { source: 'map', version: '1', authority: 'authoritative' };
  assert.throws(() => buildSdkAuthoringMetadata(JSON.stringify({ ...input, replacements: [{ symbolId: 'member', replacementSymbolId: 'member',
    sdkVersion: '2', catalogVersion: 'c1', oldText: 'a', newText: 'b', provenance }] })), /version/);
  const malformed = buildSdkAuthoringMetadata(JSON.stringify(input));
  malformed.symbols[0].authority = { status: 'known', value: 'server' };
  assert.throws(() => readApiPolicyMetadata(JSON.stringify(malformed)), /policy symbol/);
});

test('metadata readers reject array-valued enums instead of coercing them to strings', () => {
  const input = { schemaVersion: 1, sdkVersion: '1', symbols: [{ symbolId: 'member', kind: 'context' }] };
  const malformed = buildSdkAuthoringMetadata(JSON.stringify(input));
  malformed.symbols[0].kind = ['context'];
  assert.throws(() => readApiPolicyMetadata(JSON.stringify(malformed)), /policy symbol/);
});

test('builder CLI writes deterministic versioned JSON to the requested output', t => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kdf-policy-builder-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const input = path.join(directory, 'input.json'), output = path.join(directory, 'output.json');
  fs.writeFileSync(input, JSON.stringify({ schemaVersion: 1, sdkVersion: 'sdk-1', symbols: [] }));
  const command = path.resolve(__dirname, '../scripts/buildSdkAuthoringMetadata.cjs');
  execFileSync(process.execPath, [command, input, output]);
  const first = fs.readFileSync(output, 'utf8');
  assert.equal(readApiPolicyMetadata(first).defaultGrants.status, 'unknown');
  execFileSync(process.execPath, [command, input, output]);
  assert.equal(fs.readFileSync(output, 'utf8'), first);
  assert.deepEqual(fs.readdirSync(directory).sort(), ['input.json', 'output.json']);
});
