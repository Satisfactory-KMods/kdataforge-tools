'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const api = require(path.join(process.env.KDF_SCRIPT_ANALYSIS_DIST || path.resolve(__dirname, '../dist'), 'scriptAnalysis'));
const { buildSdkAuthoringMetadata } = require('../scripts/buildSdkAuthoringMetadata.cjs');
const provenance = { source: 'runtime-policy.json', version: 'runtime-1', authority: 'authoritative' };
const fact = value => ({ value, provenance });
const slot = (kind = 'content', argument = 0, fieldPath = []) => ({ argument, path: fieldPath, kind });
const symbol = (name, capabilities = [], references = [], options = {}) => ({ symbolId: name, kind: options.kind || 'context',
  binding: fact({ kind: options.export ? 'export' : 'context', module: options.module || api.SDK_MODULE,
    exportName: options.export || 'Context', members: options.export && !options.member ? [] : [options.member || name], operation: 'call' }),
  capabilities: fact(capabilities), references: fact(references) });
function metadata(extra = {}) {
  return api.readApiPolicyMetadata(JSON.stringify(buildSdkAuthoringMetadata(JSON.stringify({
    schemaVersion: 1, sdkVersion: 'sdk-1', catalogVersion: 'catalog-1', defaultGrants: fact(['read']),
    symbols: [symbol('defineHandler', [], [], { export: 'defineHandler' }),
      symbol('defineMeta', [], [], { export: 'defineMeta' }), symbol('removeItem', ['delete'], [slot()]),
      symbol('readThing', ['read']), symbol('getClass', ['read'], [slot('class', 1)]),
      symbol('getObject', ['read'], [slot('asset', 1)]), symbol('createItem', ['create'], [slot('content', 0, ['id'])]),
      symbol('newRemoveItem', ['delete'], [slot()]),
      symbol('refresh', ['reflect'], [], { kind: 'reflected', export: 'Thing', member: 'refresh', module: 'satisfactory' })],
    ...extra,
  }))));
}
const prelude = `import { defineHandler, defineMeta } from '${api.SDK_MODULE}';\n`;
const script = (body, declared = []) => prelude + `export const meta = defineMeta({ capabilities: ${JSON.stringify(declared)} });\n`
  + `export default defineHandler(async (ctx) => { ${body} });\n`;
function hostFor(files, extra = {}) {
  return {
    async realpath(name) {
      if (name === '/pack' || Object.hasOwn(files, name)) return name;
      const error = new Error('missing'); error.code = 'ENOENT'; throw error;
    },
    async read(name) {
      if (!Object.hasOwn(files, name)) throw new Error('missing');
      return { fileName: name, text: files[name], version: 1 };
    }, ...extra,
  };
}
async function analyze(files, policy = metadata(), options = {}) {
  return api.analyzeScripts({ packRoot: '/pack', entrypoints: ['main.ts'], host: hostFor(files), allPackScriptsProvided: true,
    ...options }, policy);
}

test('source getters, initializers and module bodies are parsed without execution', async () => {
  globalThis.__kdfAnalysisExecuted = false;
  const source = script(`const evil = { get id() { globalThis.__kdfAnalysisExecuted = true; throw Error('executed'); } };
    ctx.removeItem(evil.id);`) + `\nglobalThis.__kdfAnalysisExecuted = true; throw new Error('module executed');`;
  const result = await analyze({ '/pack/main.ts': source });
  assert.equal(globalThis.__kdfAnalysisExecuted, false);
  assert.equal(result.references[0].status, 'dynamic');
  assert.equal(api.planContentRename(result, { kind: 'content', oldId: 'Old', newId: 'New' }).safe, false);
  delete globalThis.__kdfAnalysisExecuted;
});

test('graph refuses escapes, encoded/absolute/bare imports and dynamic loading without reading them', async () => {
  const reads = [];
  const files = { '/pack/main.ts': `import '../outside.ts'; import '/etc/passwd'; import 'node:fs';
    import './%2e%2e/outside'; import './bad\\\\name'; const m = import('./dynamic'); require('./required');` };
  const host = hostFor(files, { async read(name) { reads.push(name); return { fileName: name, text: files[name], version: 1 }; } });
  const graph = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'], host });
  assert.equal(graph.complete, false);
  assert.deepEqual(reads, ['/pack/main.ts']);
  assert.ok(graph.issues.some(issue => issue.code === 'import-escape'));
  assert.ok(graph.issues.some(issue => issue.code === 'dynamic-code'));
});

test('filesystem host rejects symlinks outside pack and invalid UTF-8', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'kdf-analysis-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const pack = path.join(directory, 'pack');
  await fs.mkdir(pack);
  await fs.writeFile(path.join(directory, 'outside.ts'), 'throw new Error("never loaded");');
  await fs.symlink(path.join(directory, 'outside.ts'), path.join(pack, 'alias.ts'));
  await fs.writeFile(path.join(pack, 'main.ts'), `import './alias';`);
  const graph = await api.buildScriptGraph({ packRoot: pack, entrypoints: ['main.ts'] });
  assert.equal(graph.modules.size, 1);
  assert.ok(graph.issues.some(issue => issue.code === 'import-escape'));
  await fs.writeFile(path.join(pack, 'invalid.ts'), Buffer.from([0xff]));
  const invalid = await api.buildScriptGraph({ packRoot: pack, entrypoints: ['invalid.ts'] });
  assert.equal(invalid.complete, false);
  assert.equal(invalid.modules.size, 0);
});

test('all four graph limits are hard bounds', async () => {
  const oversized = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'],
    host: hostFor({ '/pack/main.ts': ' '.repeat(api.SCRIPT_LIMITS.fileBytes + 1) }) });
  assert.equal(oversized.modules.size, 0);
  assert.ok(oversized.issues.some(issue => issue.code === 'file-limit'));
  const many = Object.fromEntries(Array.from({ length: 65 }, (_, index) => [`/pack/m${index}.ts`, 'export {};']));
  const modules = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: Object.keys(many), host: hostFor(many) });
  assert.equal(modules.modules.size, 64);
  assert.equal(modules.complete, false);
  const deep = Object.fromEntries(Array.from({ length: 18 }, (_, index) => [`/pack/m${index}.ts`,
    index === 17 ? 'export {};' : `import './m${index + 1}';`]));
  const depth = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['m0.ts'], host: hostFor(deep) });
  assert.equal(depth.modules.size, 17);
  assert.ok(depth.issues.some(issue => issue.code === 'depth-limit'));
  const large = Object.fromEntries(Array.from({ length: 9 }, (_, index) => [`/pack/m${index}.ts`, ' '.repeat(api.SCRIPT_LIMITS.fileBytes)]));
  const bytes = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: Object.keys(large), host: hostFor(large) });
  assert.equal(bytes.bytes, 4 * 1024 * 1024);
  assert.equal(bytes.modules.size, 8);
  assert.ok(bytes.issues.some(issue => issue.code === 'byte-limit'));
});

test('cycles terminate, .js imports resolve TS snapshots, ambiguous candidates refuse completeness', async () => {
  const files = { '/pack/main.ts': `import { ID } from './ids.js';`, '/pack/ids.ts': `export const ID = 'Old'; import './main';` };
  const graph = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'], host: hostFor(files) });
  assert.equal(graph.complete, true);
  assert.equal(graph.modules.size, 2);
  files['/pack/ids.tsx'] = 'export const ID = "Other";';
  const ambiguous = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'], host: hostFor(files) });
  assert.equal(ambiguous.complete, false);
  assert.ok(ambiguous.issues.some(issue => issue.code === 'ambiguous-import'));
});

test('.mjs and .cjs imports preserve the corresponding TypeScript module extension', async () => {
  const files = { '/pack/main.ts': `import './esm.mjs'; import './common.cjs';`,
    '/pack/esm.mts': 'export const value = 1;', '/pack/common.cts': 'export const value = 2;' };
  const graph = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'], host: hostFor(files) });
  assert.equal(graph.complete, true, JSON.stringify(graph.issues));
  assert.deepEqual(new Set(graph.modules.keys()), new Set(Object.keys(files)));
});

test('syntax-rejected files still consume the aggregate byte budget', async () => {
  const deep = `const value = ${'('.repeat(300)}1${')'.repeat(300)};`.padEnd(api.SCRIPT_LIMITS.fileBytes, ' ');
  const files = Object.fromEntries(Array.from({ length: 9 }, (_, index) => [`/pack/m${index}.ts`, deep]));
  let reads = 0;
  const host = hostFor(files, { async read(fileName) { reads++; return { fileName, text: files[fileName], version: 1 }; } });
  const graph = await api.buildScriptGraph({ packRoot: '/pack', entrypoints: Object.keys(files), host });
  assert.equal(graph.bytes, api.SCRIPT_LIMITS.bytes);
  assert.equal(reads, 8);
  assert.equal(graph.complete, false);
  assert.ok(graph.issues.some(issue => issue.code === 'syntax-limit'));
  assert.ok(graph.issues.some(issue => issue.code === 'byte-limit'));
});

test('stale reads and cancelled generations cannot publish authoring models or edit plans', async () => {
  const generations = new api.AnalysisGeneration();
  let finishRead, readStarted;
  const started = new Promise(resolve => { readStarted = resolve; });
  const files = { '/pack/main.ts': script(`ctx.removeItem('Old');`) };
  const host = hostFor(files, { read(name) { readStarted(); return new Promise(resolve => {
    finishRead = () => resolve({ fileName: name, text: files[name], version: 1 });
  }); } });
  const pending = api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'], host, ticket: generations.next() });
  await started;
  generations.invalidate();
  finishRead();
  const stale = await pending;
  assert.equal(stale.complete, false);
  assert.equal(stale.modules.size, 0);
  const controller = new AbortController();
  const ticket = generations.next(controller.signal);
  const result = await analyze(files, metadata(), { ticket });
  controller.abort();
  assert.equal(api.buildHoverModel(result, '/pack/main.ts', files['/pack/main.ts'].indexOf('removeItem')), undefined);
  assert.equal(api.buildCapabilityLensModel(result, '/pack/main.ts'), undefined);
  assert.equal(api.planContentRename(result, { kind: 'content', oldId: 'Old', newId: 'New' }).safe, false);
});

test('Q13 context and reflected symbol aliases preserve policy facts and unknown fields', async () => {
  const source = script(`ctx.removeItem('Old');`) + `\nimport { Thing as Renamed } from 'satisfactory'; Renamed.refresh();`;
  const result = await analyze({ '/pack/main.ts': source });
  const hover = api.buildHoverModel(result, '/pack/main.ts', source.indexOf('removeItem'));
  assert.equal(hover.symbolId, 'removeItem');
  assert.deepEqual(hover.capabilities.value, ['delete']);
  assert.equal(hover.authority.status, 'unknown');
  assert.equal(hover.lifecycle.status, 'unknown');
  assert.equal(hover.quotas.status, 'unknown');
  const reflected = api.buildHoverModel(result, '/pack/main.ts', source.indexOf('refresh()'));
  assert.equal(reflected.symbolId, 'refresh');
  assert.equal(reflected.kind, 'reflected');
  assert.equal(result.complete, true, JSON.stringify(result.issues));
});

test('cancellation releases a pending host read without waiting for its completion', async () => {
  const generation = new api.AnalysisGeneration();
  let started;
  const ready = new Promise(resolve => { started = resolve; });
  const host = hostFor({ '/pack/main.ts': '' }, { read() { started(); return new Promise(() => {}); } });
  const pending = api.buildScriptGraph({ packRoot: '/pack', entrypoints: ['main.ts'], host, ticket: generation.next() });
  await ready;
  generation.invalidate();
  const result = await pending;
  assert.equal(result.complete, false);
  assert.ok(result.issues.some(issue => issue.code === 'stale-generation'));
});

test('shadowed ctx and SDK names never inherit API identity from spelling', async () => {
  const source = `function fake(ctx) { ctx.removeItem('Wrong'); }
    function defineHandler(fn) { return fn; }
    defineHandler(ctx => ctx.removeItem('Wrong'));`;
  const result = await analyze({ '/pack/main.ts': source });
  assert.equal(result.uses.length, 0);
  assert.equal(result.references.length, 0);
});

test('Q14 default grants, missing declarations, and redundant declarations stay separate', async () => {
  const result = await analyze({ '/pack/main.ts': script(`ctx.readThing(); ctx.removeItem('Old');`, ['read', 'extra']) });
  const model = api.buildCapabilityLensModel(result, '/pack/main.ts');
  assert.equal(model.complete, true, JSON.stringify(result.issues));
  assert.deepEqual(model.used, ['delete', 'read']);
  assert.deepEqual(model.missing, ['delete']);
  assert.deepEqual(model.unneeded, ['extra', 'read']);
  assert.deepEqual(model.defaultGranted, ['read']);
  assert.equal(model.automaticGrant, false);
});

test('unknown policy/defaults and dynamic meta never claim unused capabilities', async () => {
  const files = { '/pack/main.ts': script(`ctx.mystery('Old');`, ['extra']) };
  const result = await analyze(files, metadata({ defaultGrants: undefined }));
  const model = api.buildCapabilityLensModel(result, '/pack/main.ts');
  assert.equal(model.complete, false);
  assert.equal(model.missing, null);
  assert.equal(model.unneeded, null);
  assert.equal(model.unknownSymbols.length, 1);
  const hover = api.buildHoverModel(result, '/pack/main.ts', files['/pack/main.ts'].indexOf('mystery'));
  assert.equal(hover.status, 'unknown');
  assert.equal(hover.capabilities.status, 'unknown');
  const getter = await analyze({ '/pack/main.ts': prelude
    + `export const meta = defineMeta({ get capabilities() { throw Error('never execute'); } });` });
  assert.equal(api.buildCapabilityLensModel(getter, '/pack/main.ts').declared, null);
});

test('capability accounting follows imports and SDK reexports by symbol', async () => {
  const files = { '/pack/main.ts': `import { handler } from './bridge'; import { run } from './other';
    export default handler(async ctx => { run(ctx); });`,
  '/pack/bridge.ts': `export { defineHandler as handler } from '${api.SDK_MODULE}';`,
  '/pack/other.ts': `import { Context } from '${api.SDK_MODULE}'; export function run(ctx: Context) { ctx.removeItem('Old'); }` };
  const result = await analyze(files);
  const model = api.buildCapabilityLensModel(result, '/pack/main.ts');
  assert.deepEqual(model.used, ['delete']);
  assert.deepEqual(model.missing, ['delete']);
  assert.equal(model.unneeded, null, 'passing context to a helper is conservatively incomplete');
});

test('Q15 literal-slot completion filters kind/prefix, caps 200 and requires exact catalog version', async () => {
  const source = script(`ctx.removeItem('Pack/'); ctx.getClass(null, '/Game/'); ctx.getObject(null, '/Asset/');`);
  const result = await analyze({ '/pack/main.ts': source });
  const identifiers = Array.from({ length: 250 }, (_, index) => ({ id: `Pack/${index}`, kind: 'content' }));
  identifiers.unshift({ id: 'Pack/WrongKind', kind: 'asset' });
  const catalog = { sdkVersion: 'sdk-1', catalogVersion: 'catalog-1', identifiers };
  const content = api.buildCompletionCandidates(result, '/pack/main.ts', source.indexOf('Pack/') + 5, catalog);
  assert.equal(content.items.length, 200);
  assert.equal(content.isIncomplete, true);
  assert.equal(content.items[0].id, 'Pack/0');
  assert.equal(content.items[0].insertText, '"Pack/0"');
  assert.equal(content.items[0].oldText, "'Pack/'");
  assert.equal(api.buildCompletionCandidates(result, '/pack/main.ts', source.indexOf('Pack/') + 5,
    { ...catalog, catalogVersion: 'catalog-2' }).items.length, 0);
  assert.equal(api.buildCompletionCandidates(result, '/pack/main.ts', 0, catalog).items.length, 0);
  for (const [kind, id] of [['class', '/Game/Thing'], ['asset', '/Asset/Thing']]) {
    const prefix = kind === 'class' ? '/Game/' : '/Asset/';
    const model = api.buildCompletionCandidates(result, '/pack/main.ts', source.indexOf(prefix) + prefix.length,
      { ...catalog, identifiers: [{ id, kind }] });
    assert.equal(model.items[0].id, id);
  }
});

test('Q16 content rename follows imported const definitions and keeps unrelated strings intact', async () => {
  const files = { '/pack/main.ts': `import { ID as chosen } from './ids';\n` + script(`ctx.removeItem(chosen); ctx.removeItem('Old'); console.log('Old');`),
    '/pack/ids.ts': `export const ID = 'Old' as const;` };
  const result = await analyze(files);
  assert.equal(result.complete, true, JSON.stringify(result.issues));
  const plan = api.planContentRename(result, { kind: 'content', oldId: 'Old', newId: 'New' });
  assert.equal(plan.safe, true, plan.blockers.join(','));
  assert.equal(plan.edits.length, 2);
  assert.deepEqual(new Set(plan.edits.map(edit => edit.fileName)), new Set(Object.keys(files)));
  assert.ok(plan.edits.every(edit => edit.oldText === "'Old'" && edit.newText === '"New"' && edit.version === 1));
  assert.equal(plan.documents.length, 2);
  const names = api.planTypeScriptSymbolRename(result, '/pack/ids.ts', files['/pack/ids.ts'].indexOf('ID'), 'NEW_ID');
  assert.equal(names.safe, true, names.blockers.join(','));
  assert.equal(names.edits.length, 2);
  assert.ok(names.edits.every(edit => edit.newText === 'NEW_ID'));
});

test('namespace constants resolve statically, but typed function results do not acquire literal authority', async () => {
  const files = { '/pack/main.ts': `import * as ids from './ids';\n` + script(`const alias = ids; ctx.removeItem(alias.ID);`),
    '/pack/ids.ts': `export const ID = 'Old';` };
  const direct = await analyze(files);
  assert.equal(direct.references[0].status, 'static');
  assert.equal(api.planContentRename(direct, { kind: 'content', oldId: 'Old', newId: 'New' }).safe, true);
  files['/pack/main.ts'] = `import * as ids from './ids';
    function forged(): typeof ids { throw Error('never execute'); }\n` + script(`ctx.removeItem(forged().ID);`);
  const computed = await analyze(files);
  assert.equal(computed.references[0].status, 'dynamic');
  assert.equal(api.planContentRename(computed, { kind: 'content', oldId: 'Old', newId: 'New' }).safe, false);
});

test('symbol rename refuses property keys and aliased dynamic evaluators', async () => {
  const text = `const value = { Old: 1 }; const key = input(); value[key]; export {};`;
  const result = await analyze({ '/pack/main.ts': text });
  const property = api.planTypeScriptSymbolRename(result, '/pack/main.ts', text.indexOf('Old'), 'New');
  assert.equal(property.safe, false);
  assert.deepEqual(property.edits, []);
  const dynamicText = `const Old = 1; const run = eval; run('Old'); export {};`;
  const dynamic = await analyze({ '/pack/main.ts': dynamicText });
  assert.equal(dynamic.graph.complete, false);
  assert.equal(api.planTypeScriptSymbolRename(dynamic, '/pack/main.ts', dynamicText.indexOf('Old'), 'New').safe, false);
});

test('Q16 dynamic, mutable, ambiguous, incomplete, colliding and unknown-scope renames refuse all edits', async () => {
  for (const body of [`ctx.removeItem('O' + 'ld');`, 'ctx.removeItem(`O${suffix}`);',
    `let id = 'Old'; ctx.removeItem(id);`, `const id = 'Old'; const id = 'Other'; ctx.removeItem(id);`,
    `ctx.createItem({ get id() { return 'Old'; } });`, `ctx[method]('Old');`,
    `const { removeItem } = ctx; removeItem('Old');`]) {
    const result = await analyze({ '/pack/main.ts': script(body) });
    const plan = api.planContentRename(result, { kind: 'content', oldId: 'Old', newId: 'New' });
    assert.equal(plan.safe, false, body);
    assert.deepEqual(plan.edits, []);
  }
  const result = await analyze({ '/pack/main.ts': script(`ctx.removeItem('Old');`) }, metadata(), { allPackScriptsProvided: false });
  assert.equal(api.planContentRename(result, { kind: 'content', oldId: 'Old', newId: 'New' }).safe, false);
  const complete = await analyze({ '/pack/main.ts': script(`ctx.removeItem('Old');`) });
  assert.equal(api.planContentRename(complete, { kind: 'content', oldId: 'Old', newId: 'New', existingIds: ['New'] }).safe, false);
});

test('Q17 only authoritative version-exact replacement proof yields a token edit', async () => {
  const source = script(`ctx.removeItem('Old');`);
  const replacement = { symbolId: 'removeItem', replacementSymbolId: 'newRemoveItem', oldText: 'removeItem', newText: 'newRemoveItem',
    sdkVersion: 'sdk-1', catalogVersion: 'catalog-1', provenance };
  const result = await analyze({ '/pack/main.ts': source }, metadata({ replacements: [replacement] }));
  const offset = source.indexOf('removeItem');
  const versions = { sdkVersion: 'sdk-1', catalogVersion: 'catalog-1' };
  const fix = api.buildReplacementQuickFix(result, '/pack/main.ts', offset, versions);
  assert.ok(fix);
  assert.equal(fix.plan.edits[0].oldText, 'removeItem');
  assert.equal(fix.plan.edits[0].newText, 'newRemoveItem');
  assert.equal(api.buildReplacementQuickFix(result, '/pack/main.ts', offset, { ...versions, sdkVersion: 'sdk-2' }), undefined);
  assert.equal(api.buildReplacementQuickFix(result, '/pack/main.ts', offset, { ...versions, catalogVersion: 'catalog-2' }), undefined);
  for (const change of [{ provenance: { ...provenance, authority: 'informational' } }, { oldText: 'RemoveItem' },
    { newText: 'newRemoveItem(); injected()' }, { replacementSymbolId: 'readThing' }]) {
    const unproven = await analyze({ '/pack/main.ts': source }, metadata({ replacements: [{ ...replacement, ...change }] }));
    assert.equal(api.buildReplacementQuickFix(unproven, '/pack/main.ts', offset, versions), undefined);
  }
  const duplicated = await analyze({ '/pack/main.ts': source }, metadata({ replacements: [replacement, replacement] }));
  assert.equal(api.buildReplacementQuickFix(duplicated, '/pack/main.ts', offset, versions), undefined);
});
