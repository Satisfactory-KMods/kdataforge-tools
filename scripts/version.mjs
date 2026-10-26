import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const jsonManifests = [
  'package.json',
  'extension/package.json',
  'packages/kdataforgejs-types/package.json',
  'packages/kdataforgejs-cli/package.json',
];

export function getQuarter(month) {
  if (!Number.isInteger(month) || month < 0 || month > 11) throw new Error(`Invalid month ${month}`);
  return Math.floor(month / 3) + 1;
}

export function computeVersion(build, channel, date = new Date()) {
  if (!Number.isInteger(build) || build < 0) throw new Error(`Invalid build ${build}`);
  if (!['beta', 'stable'].includes(channel)) throw new Error(`Invalid channel ${channel}`);
  const base = `${date.getFullYear()}.${getQuarter(date.getMonth())}.${build}`;
  return channel === 'beta' ? `${base}-beta.0` : base;
}

function buildNumber() {
  if (process.env.GITHUB_RUN_NUMBER) return Number(process.env.GITHUB_RUN_NUMBER);
  try {
    return Number(execFileSync('git', ['rev-list', '--count', 'HEAD'], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim());
  } catch {
    return 0;
  }
}

function requestedChannel() {
  const index = process.argv.indexOf('--channel');
  if (index >= 0) return process.argv[index + 1];
  return process.env.GITHUB_REF_NAME === 'main' ? 'stable' : 'beta';
}

export function applyVersion(version) {
  for (const relative of jsonManifests) {
    const file = join(root, relative);
    const manifest = JSON.parse(readFileSync(file, 'utf8'));
    manifest.version = relative === 'extension/package.json' ? version.replace(/-beta\.0$/, '') : version;
    if (relative === 'extension/package.json') manifest.preview = version.includes('-beta.');
    writeFileSync(file, JSON.stringify(manifest, null, 2) + '\n');
  }
  const cargo = join(root, 'Cargo.toml');
  const source = readFileSync(cargo, 'utf8');
  writeFileSync(cargo, source.replace(/(\[workspace\.package\][\s\S]*?\nversion = ")[^"]+("\n)/, `$1${version}$2`));
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const version = computeVersion(buildNumber(), requestedChannel());
  if (process.argv.includes('--apply')) applyVersion(version);
  console.log(process.argv.includes('--extension') ? version.replace(/-beta\.0$/, '') : version);
}
