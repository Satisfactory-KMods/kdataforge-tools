'use strict';
const fs = require('node:fs');

/**
 * Input: JSON {schemaVersion:1,sdkVersion,catalogVersion?,defaultGrants?,symbols,replacements?}.
 * Each supplied fact is {value,provenance:{source,version,authority}}. Missing facts become
 * {status:'unknown',reason}; no capability, default, lifecycle or quota is inferred from a name.
 * Symbol facts: binding, capabilities, authority, lifecycle, quotas, references, signature,
 * deterministicInitialization. Global/instance bindings declare their globals explicitly;
 * reference slots of kind 'pack-file' may declare exact allowed extensions.
 * Output follows src/scriptAnalysis/policy.ts (ApiPolicyMetadata). This command writes only
 * the explicitly requested output; it neither imports workspace code nor scans SDK comments.
 */
function buildSdkAuthoringMetadata(json) {
  if (typeof json !== 'string' || Buffer.byteLength(json, 'utf8') > 4 * 1024 * 1024)
    throw new Error('Policy input must be JSON text of at most 4 MiB');
  const input = JSON.parse(json);
  const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
  const text = value => typeof value === 'string' && value.length > 0;
  const texts = value => Array.isArray(value) && value.every(text) && new Set(value).size === value.length;
  const scalar = value => text(value) || (typeof value === 'number' && Number.isFinite(value));
  const provenance = value => object(value) && text(value.source) && text(value.version)
    && ['authoritative', 'informational'].includes(value.authority);
  const own = (value, key) => Object.hasOwn(value, key) ? value[key] : undefined;
  const fact = (value, validate) => {
    if (value === undefined || (object(value) && value.status === 'unknown')) {
      if (value?.reason !== undefined && !text(value.reason)) throw new Error('Invalid unknown policy reason');
      return { status: 'unknown', reason: value?.reason || 'Not supplied by the policy source' };
    }
    if (!object(value) || !provenance(value.provenance) || !validate(value.value))
      throw new Error('Invalid policy fact or missing provenance');
    return { status: 'known', value: value.value, provenance: value.provenance };
  };
  if (!object(input) || input.schemaVersion !== 1 || !text(input.sdkVersion)
    || !(input.catalogVersion === undefined || input.catalogVersion === null || text(input.catalogVersion))
    || !Array.isArray(input.symbols) || input.symbols.length > 20_000)
    throw new Error('Invalid policy input header');
  const ids = new Set();
  const symbols = input.symbols.map(symbol => {
    if (!object(symbol) || !text(symbol.symbolId) || ids.has(symbol.symbolId)
      || !['context', 'reflected', 'global'].includes(symbol.kind)) throw new Error('Invalid or duplicate policy symbol');
    ids.add(symbol.symbolId);
    return {
      symbolId: symbol.symbolId, kind: symbol.kind,
      binding: fact(own(symbol, 'binding'), value => object(value) && ['context', 'export', 'global', 'instance'].includes(value.kind)
        && text(value.module) && text(value.exportName) && Array.isArray(value.members) && value.members.every(text)
        && ['call', 'read', 'write', 'construct'].includes(value.operation)),
      capabilities: fact(own(symbol, 'capabilities'), texts),
      authority: fact(own(symbol, 'authority'), scalar), lifecycle: fact(own(symbol, 'lifecycle'), scalar),
      quotas: fact(own(symbol, 'quotas'), value => Array.isArray(value) && value.every(quota => object(quota)
        && text(quota.name) && typeof quota.value === 'number' && Number.isFinite(quota.value) && quota.value >= 0 && text(quota.unit))),
      references: fact(own(symbol, 'references'), value => Array.isArray(value) && value.every(slot => object(slot)
        && Number.isSafeInteger(slot.argument) && slot.argument >= 0 && slot.argument < 64
        && Array.isArray(slot.path) && slot.path.length <= 16 && slot.path.every(text)
        && ['asset', 'class', 'content', 'pack-file'].includes(slot.kind)
        && (slot.extensions === undefined || texts(slot.extensions)))),
      signature: fact(own(symbol, 'signature'), text),
      deterministicInitialization: fact(own(symbol, 'deterministicInitialization'), value => typeof value === 'boolean'),
    };
  });
  const catalogVersion = input.catalogVersion ?? null;
  const replacements = own(input, 'replacements') ?? [];
  if (!Array.isArray(replacements)) throw new Error('Invalid replacement map');
  for (const replacement of replacements) {
    if (!object(replacement) || !ids.has(replacement.symbolId) || !ids.has(replacement.replacementSymbolId)
      || !text(replacement.oldText) || !text(replacement.newText) || !provenance(replacement.provenance)
      || replacement.sdkVersion !== input.sdkVersion || replacement.catalogVersion !== catalogVersion)
      throw new Error('Invalid or version-mismatched policy replacement');
  }
  return { schemaVersion: 1, sdkVersion: input.sdkVersion, catalogVersion,
    defaultGrants: fact(own(input, 'defaultGrants'), texts), symbols, replacements };
}

module.exports = { buildSdkAuthoringMetadata };
if (require.main === module) {
  const [, , input, output] = process.argv;
  if (!input || !output || process.argv.length !== 4)
    throw new Error('Usage: node buildSdkAuthoringMetadata.cjs INPUT.json OUTPUT.json');
  if (fs.statSync(input).size > 4 * 1024 * 1024) throw new Error('Policy input exceeds 4 MiB');
  const metadata = buildSdkAuthoringMetadata(fs.readFileSync(input, 'utf8'));
  fs.writeFileSync(output, JSON.stringify(metadata, null, 2) + '\n');
}
