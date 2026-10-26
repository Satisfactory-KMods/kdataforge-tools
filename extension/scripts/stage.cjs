const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const extension = path.resolve(__dirname, '..');
const profile = process.env.KDF_NATIVE_BUILD_PROFILE || 'release';
const cargoTarget = process.env.CARGO_TARGET_DIR ? path.resolve(process.env.CARGO_TARGET_DIR) : path.join(root, 'target');
const suffix = process.platform === 'win32' ? '.exe' : '';
const bin = path.join(extension, 'bin');
fs.mkdirSync(bin, { recursive: true });
for (const name of ['kdf', 'kdf-language-server']) {
  const source = path.join(cargoTarget, profile, name + suffix);
  if (!fs.existsSync(source)) throw new Error(`Missing ${profile} binary: ${source}`);
  fs.copyFileSync(source, path.join(bin, name + suffix));
}
fs.copyFileSync(path.join(root, 'LICENSE'), path.join(extension, 'LICENSE'));
console.log(`Staged ${profile} binaries for ${process.platform}`);
