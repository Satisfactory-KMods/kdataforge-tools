const { test } = require('node:test');
const assert = require('node:assert/strict');
const {
  newScriptContent,
  projectRef,
  renderSdkTemplate,
  scriptFilename,
} = require('../dist/sdkProjectModel');

test('project names become valid bounded pack refs', () => {
  assert.equal(projectRef('My scripted pack'), 'My-scripted-pack');
  assert.throws(() => projectRef('123'), /beginning with a letter/);
  assert.throws(() => projectRef('x'.repeat(51)), /at most 50/);
});

test('script names cannot escape the scripts directory', () => {
  assert.equal(scriptFilename('startup'), 'startup.ts');
  assert.equal(scriptFilename('startup.ts'), 'startup.ts');
  assert.throws(() => scriptFilename('../startup'), /only letters/);
  assert.throws(() => scriptFilename('a/b'), /only letters/);
});

test('SDK templates replace known placeholders and preserve unknown ones', () => {
  assert.equal(
    renderSdkTemplate('{{ref}}@{{sdkVersion}} {{future}}', { ref: 'Pack', sdkVersion: '1.2.3' }),
    'Pack@1.2.3 {{future}}',
  );
});

test('new script uses current typed SDK handler contract', () => {
  const content = newScriptContent();
  assert.match(content, /defineHandler/);
  assert.match(content, /ctx\.catalog\(\)\.digest/);
  assert.match(content, /Promise\.resolve/);
});
