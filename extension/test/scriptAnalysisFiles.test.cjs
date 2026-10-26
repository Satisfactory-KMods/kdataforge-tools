'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const api = require(path.join(process.env.KDF_SCRIPT_ANALYSIS_DIST || path.resolve(__dirname, '../dist'), 'scriptAnalysis'));
const { buildSdkAuthoringMetadata } = require('../scripts/buildSdkAuthoringMetadata.cjs');
const provenance = { source: 'file-api-contract', version: 'files-1', authority: 'authoritative' };
const fact = value => ({ value, provenance });
function policy() {
  const symbols = [];
  for (const name of ['YMLFile', 'JSONFile']) {
    const binding = (kind, members, operation) => fact({ kind, module: api.GLOBAL_MODULE, exportName: name, members, operation });
    symbols.push({ symbolId: name, kind: 'global', binding: binding('global', [], 'construct'), capabilities: fact([]),
      references: fact([{ kind: 'pack-file', argument: 0, path: [], extensions: name === 'YMLFile' ? ['.yml', '.yaml'] : ['.json'] }]),
      signature: fact(`new ${name}(path: string): ${name}`) });
    symbols.push({ symbolId: `${name}.json`, kind: 'global', binding: binding('instance', ['json'], 'call'),
      capabilities: fact([]), references: fact([]), signature: fact('json(): Promise<unknown>') });
    symbols.push({ symbolId: `${name}.save`, kind: 'global', binding: binding('instance', ['save'], 'call'),
      capabilities: fact(['kdf.files.write']), authority: fact('server'), lifecycle: fact('Runtime'),
      deterministicInitialization: fact(false), references: fact([]), signature: fact('save(value: unknown): Promise<void>') });
  }
  for (const name of ['parse', 'stringify']) symbols.push({ symbolId: `YML.${name}`, kind: 'global',
    binding: fact({ kind: 'global', module: api.GLOBAL_MODULE, exportName: 'YML', members: [name], operation: 'call' }),
    capabilities: fact([]), references: fact([]) });
  return api.readApiPolicyMetadata(JSON.stringify(buildSdkAuthoringMetadata(JSON.stringify({ schemaVersion: 1,
    sdkVersion: 'sdk-files', catalogVersion: 'catalog-files', defaultGrants: fact([]), symbols }))));
}
const packPaths = ['data/settings.yml', 'data/other.yaml', 'data/values.json'];
async function analyze(text, extra = {}) {
  const reads = [];
  const host = {
    async realpath(fileName) {
      if (fileName === '/pack' || fileName === '/pack/main.ts' || packPaths.some(name => fileName === `/pack/${name}`)) return fileName;
      throw new Error('missing');
    },
    async read(fileName) { reads.push(fileName); return { fileName, text, version: 7 }; },
  };
  const result = await api.analyzeScripts({ packRoot: '/pack', entrypoints: ['main.ts'], host,
    allPackScriptsProvided: true, declaredPackFiles: packPaths.map(path => ({ path, provenance })),
    execution: { authority: 'server', lifecycle: 'Runtime', deterministicInitialization: false }, ...extra }, policy());
  return { result, reads };
}

test('authoritative file globals support static paths, async json hover and canonical path authority without reads', async () => {
  let executed = false;
  globalThis.YMLFile = function () { executed = true; throw new Error('must not execute'); };
  const text = `async function load() { const file = new YMLFile('data/settings.yml');
    const value = await file.json(); const other = await new JSONFile('data/values.json').json(); }`;
  const { result, reads } = await analyze(text);
  assert.equal(executed, false);
  delete globalThis.YMLFile;
  assert.equal(result.complete, true, JSON.stringify(result.issues));
  assert.deepEqual(reads, ['/pack/main.ts']);
  const paths = result.references.filter(reference => reference.kind === 'pack-file');
  assert.equal(paths.length, 2);
  assert.ok(paths.every(reference => reference.status === 'static' && reference.pathAuthority.fileName.startsWith('/pack/data/')));
  const hover = api.buildHoverModel(result, '/pack/main.ts', text.indexOf('file.json') + 5);
  assert.equal(hover.symbolId, 'YMLFile.json');
  assert.equal(hover.signature.value, 'json(): Promise<unknown>');
  const fileHover = api.buildPackFileHoverModel(result, '/pack/main.ts', text.indexOf('data/settings'));
  assert.equal(fileHover.path, 'data/settings.yml');
  assert.equal(fileHover.authority.fileName, '/pack/data/settings.yml');
  assert.deepEqual(fileHover.authority.provenance, provenance);
});

test('file completion offers only declared canonical paths of the authoritative file type', async () => {
  const text = `new YMLFile('data/'); new JSONFile('data/');`;
  const { result } = await analyze(text);
  const catalog = { sdkVersion: 'sdk-files', catalogVersion: 'catalog-files',
    identifiers: [{ id: 'data/undeclared.yml', kind: 'pack-file' }, { id: '../escape.yml', kind: 'pack-file' }] };
  const yml = api.buildCompletionCandidates(result, '/pack/main.ts', text.indexOf('data/') + 5, catalog);
  assert.deepEqual(yml.items.map(item => item.id), ['data/settings.yml', 'data/other.yaml']);
  const json = api.buildCompletionCandidates(result, '/pack/main.ts', text.lastIndexOf('data/') + 5, catalog);
  assert.deepEqual(json.items.map(item => item.id), ['data/values.json']);
});

test('dynamic/nonliteral, undeclared, wrong-type and escaping paths mark analysis incomplete', async () => {
  for (const argument of [`filename`, `'data/' + 'settings.yml'`, '`data/${name}.yml`',
    `'../secret.yml'`, `'/etc/secret.yml'`, `'C:/secret.yml'`, `'data/%2e%2e/secret.yml'`,
    `'data/missing.yml'`, `'data/values.json'`, `({ get path() { throw Error('must not execute'); } }).path`]) {
    const text = `const filename = 'data/settings.yml'; new YMLFile(${argument});`;
    const { result } = await analyze(text);
    assert.equal(result.complete, false, argument);
    assert.ok(result.references.some(reference => ['dynamic', 'invalid'].includes(reference.status)), argument);
    assert.ok(result.references.every(reference => !reference.pathAuthority), argument);
  }
  const absent = await analyze('new YMLFile();');
  assert.ok(absent.result.issues.some(issue => issue.code === 'missing-reference-argument'));
});

test('file path symlink escapes never acquire authority or enter completion', async () => {
  const { result } = await analyze(`new YMLFile('data/settings.yml');`, {
    declaredPackFiles: [{ path: 'data/settings.yml', provenance }],
    host: { async realpath(name) { return name.endsWith('settings.yml') ? '/outside/settings.yml' : name; },
      async read(fileName) { return { fileName, text: `new YMLFile('data/settings.yml');`, version: 1 }; } },
  });
  assert.equal(result.graph.packFiles.size, 0);
  assert.equal(result.complete, false);
  assert.ok(result.graph.issues.some(issue => issue.code === 'import-escape'));
  assert.equal(result.references[0].pathAuthority, undefined);
});

test('global spelling alone and shadowed constructors/methods do not confer SDK authority', async () => {
  const text = `function f(YMLFile, JSONFile, YML) {
    new YMLFile('data/settings.yml').json(); new JSONFile('data/values.json').save({}); YML.parse('x'); }`;
  const { result } = await analyze(text);
  assert.equal(result.references.length, 0);
  assert.equal(result.uses.length, 0);
  const noGlobals = api.readApiPolicyMetadata(JSON.stringify(buildSdkAuthoringMetadata(JSON.stringify({
    schemaVersion: 1, sdkVersion: 'sdk-files', catalogVersion: 'catalog-files', symbols: [] }))));
  const noPolicy = api.analyzeScriptGraph(result.graph, noGlobals);
  assert.equal(noPolicy.uses.length, 0);
});

test('YML.parse/stringify are policy-proven pure operations requiring no capability', async () => {
  const text = `const decoded = YML.parse('value: 1'); const encoded = YML.stringify(decoded);`;
  const { result } = await analyze(text, { execution: { authority: 'client', lifecycle: 'Initialization', deterministicInitialization: true } });
  assert.equal(result.complete, true, JSON.stringify(result.issues));
  const lens = api.buildCapabilityLensModel(result, '/pack/main.ts');
  assert.deepEqual(lens.used, []);
  assert.deepEqual(lens.missing, []);
  assert.equal(lens.complete, true);
  assert.deepEqual(api.buildHoverModel(result, '/pack/main.ts', text.indexOf('parse')).capabilities.value, []);
});

test('both save methods require write capability and server Runtime authority', async () => {
  const text = `async function save() { const y = new YMLFile('data/settings.yml'); await y.save({});
    await new JSONFile('data/values.json').save({}); }`;
  const { result } = await analyze(text);
  assert.equal(result.complete, true, JSON.stringify(result.issues));
  const lens = api.buildCapabilityLensModel(result, '/pack/main.ts');
  assert.deepEqual(lens.used, ['kdf.files.write']);
  assert.deepEqual(lens.missing, ['kdf.files.write']);
  assert.equal(lens.automaticGrant, false);
  const hover = api.buildHoverModel(result, '/pack/main.ts', text.indexOf('y.save') + 2);
  assert.equal(hover.symbolId, 'YMLFile.save');
  assert.equal(hover.authority.value, 'server');
  assert.equal(hover.lifecycle.value, 'Runtime');
  assert.equal(hover.deterministicInitialization.value, false);
  for (const [execution, expected] of [
    [{ authority: 'client', lifecycle: 'Runtime', deterministicInitialization: false }, 'authority-mismatch'],
    [{ authority: 'server', lifecycle: 'Initialization', deterministicInitialization: true }, 'lifecycle-mismatch'],
    [{ authority: 'server', lifecycle: 'Runtime', deterministicInitialization: true }, 'deterministic-initialization-forbidden'],
    [{}, 'unknown-execution-authority'],
  ]) {
    const { result: invalid } = await analyze(text, { execution });
    assert.equal(invalid.complete, false);
    assert.ok(invalid.issues.some(issue => issue.code === expected), JSON.stringify(invalid.issues));
    assert.deepEqual(api.buildCapabilityLensModel(invalid, '/pack/main.ts').missing, ['kdf.files.write']);
  }
});

test('constructor and instance const aliases preserve identity; escaping instances prevent completeness', async () => {
  const { result } = await analyze(`const File = YMLFile; const y = new File('data/settings.yml'); const alias = y; alias.save({});`);
  assert.equal(result.complete, true, JSON.stringify(result.issues));
  assert.deepEqual(api.buildCapabilityLensModel(result, '/pack/main.ts').used, ['kdf.files.write']);
  const escaped = await analyze(`function make() { return new YMLFile('data/settings.yml'); } make().save({});`);
  assert.equal(escaped.result.complete, false);
  assert.ok(escaped.result.issues.some(issue => issue.code === 'api-instance-escape'));
});
