'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { createContentEditPlan, combineContentRenamePlans, validateContentEditPlan } = require(
  path.join(process.env.KDF_SCRIPT_ANALYSIS_DIST || path.resolve(__dirname, '../dist'), 'contentRename'));
const documents = [{ fileName: '/pack/main.ts', text: '"Old"', version: 2 },
  { fileName: '/pack/items.yml', text: 'id: Old', version: 'hash-1' },
  { fileName: '/pack/other.ts', text: 'export {};', version: 4 }];
const scriptEdit = { fileName: '/pack/main.ts', start: 0, end: 5, oldText: '"Old"', newText: '"New"', version: 2 };
const yamlEdit = { fileName: '/pack/items.yml', start: 4, end: 7, oldText: 'Old', newText: 'New', version: 'hash-1' };

test('combines TS and YAML edits with exact text, versions and one generation', () => {
  const script = createContentEditPlan(3, [documents[0], documents[2]], [scriptEdit]);
  const yaml = createContentEditPlan(3, [documents[1]], [yamlEdit]);
  const plan = combineContentRenamePlans([script, yaml], documents, 3);
  assert.equal(plan.safe, true);
  assert.equal(plan.edits.length, 2);
  assert.equal(plan.documents.length, 3);
  assert.equal(combineContentRenamePlans([script, script, yaml], documents, 3).edits.length, 2);
});

test('any refused provider blocks the whole combined edit', () => {
  const script = createContentEditPlan(3, documents, [scriptEdit], ['dynamic-reference']);
  const yaml = createContentEditPlan(3, [documents[1]], [yamlEdit]);
  const plan = combineContentRenamePlans([script, yaml], documents, 3);
  assert.equal(plan.safe, false);
  assert.deepEqual(plan.edits, []);
});

test('stale versions including unedited files, changed old text and stale generation reject atomically', () => {
  const plan = createContentEditPlan(3, documents, [scriptEdit, yamlEdit]);
  for (const current of [documents.map(document => ({ ...document, version: String(document.version) + '-changed' })),
    documents.map(document => document.fileName.endsWith('other.ts') ? { ...document, version: 5 } : document),
    documents.map(document => ({ ...document, text: document.text.replace('Old', 'Bad') }))]) {
    const stale = validateContentEditPlan(plan, current, 3);
    assert.equal(stale.safe, false);
    assert.deepEqual(stale.edits, []);
  }
  assert.equal(validateContentEditPlan(plan, documents, 4).safe, false);
});

test('overlaps, conflicting replacements, out-of-bounds edits and duplicate snapshots reject', () => {
  const first = createContentEditPlan(3, documents, [scriptEdit]);
  const conflict = createContentEditPlan(3, documents, [{ ...scriptEdit, newText: '"Another"' }]);
  assert.equal(combineContentRenamePlans([first, conflict], documents, 3).safe, false);
  assert.equal(createContentEditPlan(3, documents, [scriptEdit,
    { ...scriptEdit, start: 1, end: 4, oldText: 'Old' }]).safe, false);
  assert.equal(createContentEditPlan(3, documents, [{ ...scriptEdit, end: 50 }]).safe, false);
  assert.equal(createContentEditPlan(3, [...documents, documents[0]], [scriptEdit]).safe, false);
});
