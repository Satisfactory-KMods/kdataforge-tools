const fs = require('node:fs');
const path = require('node:path');

const extension = path.resolve(__dirname, '..');
const root = path.resolve(extension, '..');
const types = path.join(root, 'packages/kdataforgejs-types');
const template = path.join(root, 'packages/kdataforgejs-cli/templates/default');
const target = path.join(extension, 'sdk/kdataforgejs');
const manifest = JSON.parse(fs.readFileSync(path.join(types, 'package.json'), 'utf8'));

fs.rmSync(target, { recursive: true, force: true });
fs.mkdirSync(path.join(target, 'types/lib'), { recursive: true });
fs.cpSync(template, path.join(target, 'template'), { recursive: true });
for (const [from, to] of [
  ['index.d.ts', 'types/index.d.ts'],
  ['lib/kdfjs-es2020.d.ts', 'types/lib/kdfjs-es2020.d.ts'],
  ['tsconfig.base.json', 'types/tsconfig.base.json'],
  ['authoring.kdfjs.json', 'authoring-metadata.json'],
]) fs.copyFileSync(path.join(types, from), path.join(target, to));
fs.writeFileSync(path.join(target, 'types/package.json'), JSON.stringify({
  name: '@sf-kmods/kdataforgejs', version: manifest.version, private: true, types: 'index.d.ts',
  exports: { './tsconfig.base.json': './tsconfig.base.json', './package.json': './package.json' },
}, null, 2) + '\n');
fs.writeFileSync(path.join(target, 'manifest.json'), JSON.stringify({ version: manifest.version }, null, 2) + '\n');
console.log(`Synced KDataForgeJS SDK ${manifest.version}`);
