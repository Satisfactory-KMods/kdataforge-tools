'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

require.extensions['.ts'] = (module, filename) => {
  const source = fs.readFileSync(filename, 'utf8');
  const output = ts.transpileModule(source, { fileName: filename,
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  module._compile(output, filename);
};
const source = name => require(path.join(__dirname, '../src', `${name}.ts`));

test('form CSP uses local resources and rejects injection', () => {
  const { contentFormHtml } = source('contentForms/webview');
  const uri = 'https://file+.vscode-resource.vscode-cdn.net/media/content-form.js';
  const html = contentFormHtml('https://*.vscode-cdn.net', 'abcdefghijklmnop', uri, uri);
  assert.match(html, /default-src 'none'/);
  assert.match(html, /connect-src 'none'/);
  assert.match(html, /script-src 'nonce-abcdefghijklmnop'/);
  assert.throws(() => contentFormHtml('https://*.vscode-cdn.net', 'abcdefghijklmnop', 'https://evil.example/a', uri));
  assert.throws(() => contentFormHtml('https://*.vscode-cdn.net', 'abcdefghijklmnop', `${uri}" onload="x`, uri));
});

test('bounded form message and minimal stale-safe YAML edit', () => {
  const { buildFormModel, validateFormMessage, planFormEdit } = source('contentForms/formModel');
  const schemas = { item: { properties: { type: { type: 'string' }, name: { type: 'string', maxLength: 8 },
    count: { type: 'integer', minimum: 0, maximum: 10 } } } };
  const text = '# keep\ntype: item\nname: old # comment\ncount: 2\nother: untouched\n';
  const uri = 'file:///pack/item.yml';
  const model = buildFormModel(text, uri, 7, 0, schemas);
  assert.equal(model.kind, 'item');
  assert.ok(model.fields.some(field => field.label === 'name'));
  const message = { uri, version: 7, documentIndex: 0, path: ['name'], operation: 'set', value: 'new' };
  const edit = planFormEdit(text, uri, 7, schemas, message);
  assert.equal(text.slice(0, edit.start) + edit.text + text.slice(edit.end), text.replace('name: old', 'name: new'));
  assert.equal(validateFormMessage({ ...message, extra: true }), null);
  assert.equal(validateFormMessage({ ...message, path: ['__proto__', ...Array(20).fill('x')] }), null);
  assert.throws(() => planFormEdit(text, uri, 8, schemas, message), /stale/);
  assert.throws(() => planFormEdit(text, uri, 7, schemas, { ...message, value: 'too long!' }), /Unsupported/);
  assert.throws(() => planFormEdit(text, uri, 7, schemas, { ...message, path: ['other'] }), /Unsupported/);
  assert.throws(() => buildFormModel(text, uri, 7, 0, {}), /Unsupported/);
});

test('form follows bundled schema references into existing array entries', () => {
  const { buildFormModel, planFormEdit } = source('contentForms/formModel');
  const schemas = {
    asset: { properties: { assets: { type: 'array', items: { $ref: 'https://schemas.local/common#/$defs/entry' } } } },
    common: { $id: 'https://schemas.local/common', $defs: { entry: { type: 'object',
      properties: { file: { type: 'string', pattern: '\\.png$' } } } } },
  };
  const text = 'type: asset\nassets:\n  - file: old.png # retain\n';
  const uri = 'file:///pack/icons.asset.yml';
  const field = buildFormModel(text, uri, 2, 0, schemas).fields.find(item => item.label === 'assets.0.file');
  assert.deepEqual(field.path, ['assets', 0, 'file']);
  const message = { uri, version: 2, documentIndex: 0, path: field.path, operation: 'set', value: 'new.png' };
  const edit = planFormEdit(text, uri, 2, schemas, message);
  assert.equal(text.slice(0, edit.start) + edit.text + text.slice(edit.end), text.replace('old.png', 'new.png'));
  assert.throws(() => planFormEdit(text, uri, 2, schemas, { ...message, value: 'new.jpg' }), /Unsupported/);
  const pack = buildFormModel('name: Example\n', 'file:///pack/pack.yml', 1, 0,
    { pack: { properties: { name: { type: 'string' } } } });
  assert.equal(pack.kind, 'pack');
});

test('localized preview uses exact namespace/key and explicit source fallback', () => {
  const { previewText, previewIcon, MAX_PREVIEW_PNG_BYTES } = source('contentPreview');
  const sources = [{ namespace: 'Pack', entries: { Key: { text: 'Local value' } } }];
  assert.deepEqual(previewText({ ns: 'Pack', key: 'Key', source: 'Fallback' }, sources),
    { state: 'resolved', text: 'Local value', origin: 'localization' });
  assert.deepEqual(previewText({ namespace: 'Other', key: 'Key', source: 'Fallback' }, sources),
    { state: 'resolved', text: 'Fallback', origin: 'source' });
  assert.deepEqual(previewText({ key: 'Key', source: 'Fallback' }, sources),
    { state: 'resolved', text: 'Fallback', origin: 'source' });
  assert.match(previewText({ stringTable: '/Game/T', key: 'K' }, sources).reason, /unavailable/);
  const png = 'data:image/png;base64,' + fs.readFileSync(path.join(__dirname,
    '../media/icon.png')).toString('base64');
  const icon = { archiveBytes: 100, entryPath: 'images/a.png', dataUrl: png, verified: true };
  assert.ok(previewIcon(icon));
  assert.equal(previewIcon({ ...icon, entryPath: 'images/../a.png' }), null);
  assert.equal(previewIcon({ ...icon, archiveBytes: 2 * 1024 * 1024 * 1024 + 1 }), null);
  assert.equal(previewIcon({ ...icon, dataUrl: 'data:image/png;base64,' + 'A'.repeat(MAX_PREVIEW_PNG_BYTES * 2) }), null);
  assert.equal(previewIcon({ ...icon, verified: false }), null);
  const damaged = Buffer.from(png.slice('data:image/png;base64,'.length), 'base64');
  damaged[damaged.length - 9] ^= 1;
  assert.equal(previewIcon({ ...icon, dataUrl: 'data:image/png;base64,' + damaged.toString('base64') }), null);
});

test('JSON and SARIF adapters sort diagnostics deterministically', () => {
  const { exportDiagnosticsJson, exportDiagnosticsSarif } = source('diagnosticExport');
  const rows = [
    { file: 'b.yml', code: 'b', message: 'B', severity: 'warning', line: 4, column: 2, yamlPath: 'x' },
    { file: 'a.yml', code: 'a', message: 'A', severity: 'error', line: 1, column: 1 },
  ];
  assert.equal(exportDiagnosticsJson(rows), exportDiagnosticsJson([...rows].reverse()));
  assert.equal(exportDiagnosticsSarif(rows), exportDiagnosticsSarif([...rows].reverse()));
  const sarif = JSON.parse(exportDiagnosticsSarif(rows));
  assert.deepEqual(sarif.runs[0].tool.driver.rules.map(rule => rule.id), ['a', 'b']);
  assert.equal(sarif.runs[0].results[0].locations[0].physicalLocation.region.startLine, 1);
  assert.equal(JSON.parse(exportDiagnosticsSarif([{ ...rows[0], advisory: 'hint' }])).runs[0].results[0].level,
    'note');
});

test('explanation refuses guessed causes', () => {
  const { explainDiagnostic } = source('explainDiagnostic');
  const input = { severity: 'error', code: 'schema.required', message: 'Missing key', evidence: [] };
  assert.equal(explainDiagnostic(input).knownCause, false);
  assert.match(explainDiagnostic(input).explanation, /unknown/);
  assert.equal(explainDiagnostic({ ...input, evidence: [{ kind: 'cause', detail: 'Provided evidence' }] }).explanation,
    'Provided evidence');
});

test('offline docs search keeps exact anchors, punctuation and bounds; excludes retired routes', () => {
  const { searchBundledDocs } = source('contentForms/docsSearch');
  const section = (anchor, text) => ({ anchor, heading: anchor || 'Page', level: anchor ? 2 : 1, text });
  const index = { pages: [
    { path: 'features/config', title: 'Config', description: '', sections: [section('', ''), section('typed', 'cfg:session_key exact code')] },
    { path: 'legacy/installer', title: 'Old installer', description: '', sections: [section('', 'cfg:session_key')] },
  ] };
  const hits = searchBundledDocs(index, 'cfg:session_key', 100);
  assert.ok(hits.length <= 20);
  assert.equal(hits[0].ref, 'features/config#typed');
  assert.match(hits[0].snippet, /cfg:session_key/);
  assert.ok(hits.every(hit => !hit.ref.startsWith('legacy/')));
  assert.deepEqual(searchBundledDocs(index, 'x'.repeat(257)), []);
});
