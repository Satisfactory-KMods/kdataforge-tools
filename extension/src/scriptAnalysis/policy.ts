export interface Provenance {
  readonly source: string;
  readonly version: string;
  readonly authority: 'authoritative' | 'informational';
}
export type PolicyFact<T> = { readonly status: 'known'; readonly value: T; readonly provenance: Provenance }
  | { readonly status: 'unknown'; readonly reason: string };
export type ReferenceKind = 'asset' | 'class' | 'content' | 'pack-file';
export type ApiOperation = 'call' | 'read' | 'write' | 'construct';
export interface ApiBinding {
  readonly kind: 'context' | 'export' | 'global' | 'instance';
  readonly module: string;
  readonly exportName: string;
  readonly members: readonly string[];
  readonly operation: ApiOperation;
}
export interface ReferenceSlot {
  readonly argument: number;
  readonly path: readonly string[];
  readonly kind: ReferenceKind;
  readonly extensions?: readonly string[];
}
export interface ApiQuota { readonly name: string; readonly value: number; readonly unit: string; }
export interface ApiPolicy {
  readonly symbolId: string;
  readonly kind: 'context' | 'reflected' | 'global';
  readonly binding: PolicyFact<ApiBinding>;
  readonly capabilities: PolicyFact<readonly string[]>;
  readonly authority: PolicyFact<string | number>;
  readonly lifecycle: PolicyFact<string | number>;
  readonly quotas: PolicyFact<readonly ApiQuota[]>;
  readonly references: PolicyFact<readonly ReferenceSlot[]>;
  readonly signature: PolicyFact<string>;
  readonly deterministicInitialization: PolicyFact<boolean>;
}
export interface ApiReplacement {
  readonly symbolId: string;
  readonly replacementSymbolId: string;
  /** Exact source token, including quotes for string-literal replacements. */
  readonly oldText: string;
  readonly newText: string;
  readonly sdkVersion: string;
  readonly catalogVersion: string | null;
  readonly provenance: Provenance;
}
export interface ApiPolicyMetadata {
  readonly schemaVersion: 1;
  readonly sdkVersion: string;
  /** null means catalog provenance is unavailable, never a wildcard version. */
  readonly catalogVersion: string | null;
  readonly defaultGrants: PolicyFact<readonly string[]>;
  readonly symbols: readonly ApiPolicy[];
  readonly replacements: readonly ApiReplacement[];
}
export type PolicyInputFact<T> = { readonly value: T; readonly provenance: Provenance }
  | { readonly status: 'unknown'; readonly reason?: string };
type InputFacts<T> = { readonly [K in keyof T]?: T[K] extends PolicyFact<infer V> ? PolicyInputFact<V> : never };
export type ApiPolicyInput = Pick<ApiPolicy, 'symbolId' | 'kind'> & InputFacts<Omit<ApiPolicy, 'symbolId' | 'kind'>>;
/** JSON-serialize this input for buildSdkAuthoringMetadata.cjs; omitted facts stay unknown. */
export interface ApiPolicyMetadataInput {
  readonly schemaVersion: 1;
  readonly sdkVersion: string;
  readonly catalogVersion?: string | null;
  readonly defaultGrants?: PolicyInputFact<readonly string[]>;
  readonly symbols: readonly ApiPolicyInput[];
  readonly replacements?: readonly ApiReplacement[];
}
export interface PolicyVersions { readonly sdkVersion: string; readonly catalogVersion: string | null; }

export function unknownPolicy<T>(reason = 'Not supplied by the policy source'): PolicyFact<T> {
  return { status: 'unknown', reason };
}
export function authoritative<T>(fact: PolicyFact<T>): fact is Extract<PolicyFact<T>, { status: 'known' }> {
  return fact.status === 'known' && fact.provenance.authority === 'authoritative';
}
export function samePolicyVersion(metadata: PolicyVersions, current: PolicyVersions): boolean {
  return metadata.sdkVersion === current.sdkVersion && metadata.catalogVersion === current.catalogVersion;
}
export function bindingKey(binding: ApiBinding): string {
  return JSON.stringify([binding.kind, binding.module, binding.exportName, binding.members, binding.operation]);
}

type RecordValue = Record<string, unknown>;
const record = (value: unknown): value is RecordValue => value !== null && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === 'string' && value.length > 0;
const texts = (value: unknown): boolean => Array.isArray(value) && value.every(text) && new Set(value).size === value.length;
const scalar = (value: unknown): boolean => text(value) || (typeof value === 'number' && Number.isFinite(value));
const oneOf = (value: unknown, choices: readonly string[]): boolean => typeof value === 'string' && choices.includes(value);
const provenance = (value: unknown): boolean => record(value) && text(value.source) && text(value.version)
  && oneOf(value.authority, ['authoritative', 'informational']);
const fact = (value: unknown, validate: (value: unknown) => boolean): boolean => record(value)
  && (value.status === 'unknown' ? text(value.reason)
    : value.status === 'known' && provenance(value.provenance) && validate(value.value));
const binding = (value: unknown): boolean => record(value) && oneOf(value.kind, ['context', 'export', 'global', 'instance'])
  && text(value.module) && text(value.exportName) && Array.isArray(value.members) && value.members.every(text)
  && oneOf(value.operation, ['call', 'read', 'write', 'construct']);
const references = (value: unknown): boolean => Array.isArray(value) && value.every(slot => record(slot)
  && Number.isSafeInteger(slot.argument) && Number(slot.argument) >= 0 && Number(slot.argument) < 64
  && Array.isArray(slot.path) && slot.path.length <= 16 && slot.path.every(text)
  && oneOf(slot.kind, ['asset', 'class', 'content', 'pack-file'])
  && (slot.extensions === undefined || texts(slot.extensions)));
const quotas = (value: unknown): boolean => Array.isArray(value) && value.every(quota => record(quota)
  && text(quota.name) && typeof quota.value === 'number' && Number.isFinite(quota.value) && quota.value >= 0 && text(quota.unit));

/** JSON is the data boundary: supplied accessors, functions and prototypes are never inspected. */
export function readApiPolicyMetadata(json: string): ApiPolicyMetadata {
  if (typeof json !== 'string' || Buffer.byteLength(json, 'utf8') > 4 * 1024 * 1024)
    throw new Error('Policy must be JSON text of at most 4 MiB');
  const data: unknown = JSON.parse(json);
  if (!record(data) || data.schemaVersion !== 1 || !text(data.sdkVersion)
    || !(data.catalogVersion === null || text(data.catalogVersion)) || !fact(data.defaultGrants, texts)
    || !Array.isArray(data.symbols) || data.symbols.length > 20_000 || !Array.isArray(data.replacements))
    throw new Error('Invalid policy metadata header');
  const ids = new Set<string>();
  for (const symbol of data.symbols) {
    if (!record(symbol) || !text(symbol.symbolId) || ids.has(symbol.symbolId)
      || !oneOf(symbol.kind, ['context', 'reflected', 'global']) || !fact(symbol.binding, binding)
      || !fact(symbol.capabilities, texts) || !fact(symbol.authority, scalar) || !fact(symbol.lifecycle, scalar)
      || !fact(symbol.quotas, quotas) || !fact(symbol.references, references)
      || !fact(symbol.signature, text) || !fact(symbol.deterministicInitialization, value => typeof value === 'boolean'))
      throw new Error('Invalid or duplicate policy symbol');
    ids.add(symbol.symbolId);
  }
  for (const replacement of data.replacements) {
    if (!record(replacement) || !text(replacement.symbolId) || !ids.has(replacement.symbolId)
      || !text(replacement.replacementSymbolId) || !ids.has(replacement.replacementSymbolId)
      || !text(replacement.oldText) || !text(replacement.newText) || !provenance(replacement.provenance)
      || replacement.sdkVersion !== data.sdkVersion || replacement.catalogVersion !== data.catalogVersion)
      throw new Error('Invalid or version-mismatched policy replacement');
  }
  return data as unknown as ApiPolicyMetadata;
}
