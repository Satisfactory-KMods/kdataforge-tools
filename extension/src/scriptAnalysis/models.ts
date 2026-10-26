import * as ts from 'typescript';
import { ContentEditPlan } from '../contentRename';
import { analysisIsCurrent, ApiUse, ScriptAnalysis, SourceRange } from './analyzer';
import { ApiPolicy, authoritative, bindingKey, PolicyFact, PolicyVersions, ReferenceKind, samePolicyVersion, unknownPolicy } from './policy';
import { editForRange, scriptEditPlan } from './rename';
import { PackFileAuthority } from './graph';

function useAt(analysis: ScriptAnalysis, fileName: string, offset: number): ApiUse | undefined {
  return analysis.uses.filter(use => use.expressionRange.fileName === fileName
    && use.expressionRange.start <= offset && offset < use.expressionRange.end)
    .sort((left, right) => left.expressionRange.end - left.expressionRange.start - (right.expressionRange.end - right.expressionRange.start))[0];
}

export interface ApiHoverModel {
  readonly range: SourceRange;
  readonly symbolId?: string;
  readonly label: string;
  readonly kind: 'context' | 'reflected' | 'global';
  readonly status: 'known' | 'unknown' | 'ambiguous';
  readonly sdkVersion: string;
  readonly catalogVersion: string | null;
  readonly capabilities: ApiPolicy['capabilities'];
  readonly defaultGrants: PolicyFact<readonly string[]>;
  readonly authority: ApiPolicy['authority'];
  readonly lifecycle: ApiPolicy['lifecycle'];
  readonly quotas: ApiPolicy['quotas'];
  readonly signature: ApiPolicy['signature'];
  readonly deterministicInitialization: ApiPolicy['deterministicInitialization'];
}

/** Q13. Plain data only; the host renders/escapes it and never enables trusted Markdown commands. */
export function buildHoverModel(analysis: ScriptAnalysis, fileName: string, offset: number): ApiHoverModel | undefined {
  if (!analysisIsCurrent(analysis)) return;
  const use = useAt(analysis, fileName, offset);
  if (!use) return;
  return { range: use.range, symbolId: use.symbolId,
    label: [use.binding.exportName, ...use.binding.members].join('.'),
    kind: use.policy?.kind ?? (use.binding.kind === 'context' ? 'context' : use.binding.kind === 'export' ? 'reflected' : 'global'), status: use.status,
    sdkVersion: analysis.metadata.sdkVersion, catalogVersion: analysis.metadata.catalogVersion,
    capabilities: use.policy?.capabilities ?? unknownPolicy(), defaultGrants: analysis.metadata.defaultGrants,
    authority: use.policy?.authority ?? unknownPolicy(), lifecycle: use.policy?.lifecycle ?? unknownPolicy(),
    quotas: use.policy?.quotas ?? unknownPolicy(), signature: use.policy?.signature ?? unknownPolicy(),
    deterministicInitialization: use.policy?.deterministicInitialization ?? unknownPolicy() };
}

export interface PackFileHoverModel {
  readonly range: SourceRange;
  readonly path?: string;
  readonly status: 'static' | 'dynamic' | 'ambiguous' | 'invalid';
  readonly authority?: PackFileAuthority;
  readonly reason?: string;
}

/** File contents are never loaded for hover. Authority is a declared, canonical pack-local path. */
export function buildPackFileHoverModel(analysis: ScriptAnalysis, fileName: string, offset: number): PackFileHoverModel | undefined {
  if (!analysisIsCurrent(analysis)) return;
  const reference = analysis.references.find(value => value.kind === 'pack-file' && value.range.fileName === fileName
    && value.range.start <= offset && offset < value.range.end);
  return reference && { range: reference.range, path: reference.value, status: reference.status,
    authority: reference.pathAuthority, reason: reference.reason };
}

export interface CapabilityLensModel {
  readonly fileName: string;
  readonly generation: number;
  readonly declared: readonly string[] | null;
  readonly used: readonly string[];
  readonly defaultGranted: readonly string[] | null;
  readonly missing: readonly string[] | null;
  readonly unneeded: readonly string[] | null;
  readonly unknownSymbols: readonly string[];
  readonly complete: boolean;
  readonly automaticGrant: false;
}

/** Q14. null means unproven, not an empty set. Default grants never become declaration edits. */
export function buildCapabilityLensModel(analysis: ScriptAnalysis, fileName: string): CapabilityLensModel | undefined {
  if (!analysisIsCurrent(analysis) || !analysis.graph.modules.has(fileName)) return;
  const reachable = new Set([fileName]);
  for (const current of reachable) for (const edge of analysis.graph.edges)
    if (edge.from === current && edge.to) reachable.add(edge.to);
  const unknownSymbols = new Set<string>(), used = new Set<string>();
  for (const use of analysis.uses) {
    if (!reachable.has(use.range.fileName)) continue;
    if (!use.policy || !authoritative(use.policy.capabilities))
      unknownSymbols.add(use.symbolId ?? [use.binding.module, use.binding.exportName, ...use.binding.members].join(':'));
    else for (const capability of use.policy.capabilities.value) used.add(capability);
  }
  const declaration = analysis.declaredCapabilities.get(fileName);
  const declared = declaration && authoritative(declaration) ? [...declaration.value].sort() : null;
  const defaults = authoritative(analysis.metadata.defaultGrants) ? [...analysis.metadata.defaultGrants.value].sort() : null;
  const complete = analysis.complete && !unknownSymbols.size && declared !== null && defaults !== null;
  return { fileName, generation: analysis.graph.ticket.generation, declared, used: [...used].sort(),
    defaultGranted: defaults,
    missing: declared && defaults ? [...used].filter(value => !declared.includes(value) && !defaults.includes(value)).sort() : null,
    unneeded: complete ? declared!.filter(value => !used.has(value) || defaults!.includes(value)) : null,
    unknownSymbols: [...unknownSymbols].sort(), complete, automaticGrant: false };
}

export interface KnownIdentifier { readonly id: string; readonly kind: ReferenceKind; readonly label?: string; }
export interface IdentifierCatalog extends PolicyVersions { readonly identifiers: readonly KnownIdentifier[]; }
export interface CompletionCandidate {
  readonly label: string;
  readonly id: string;
  readonly kind: ReferenceKind;
  readonly insertText: string;
  readonly range: SourceRange;
  readonly oldText: string;
  readonly version: string | number;
}
export interface ScriptCompletions { readonly items: readonly CompletionCandidate[]; readonly isIncomplete: boolean; }

/** Q15. Candidates only at proven reference slots; replaces one quoted literal, never arbitrary code. */
export function buildCompletionCandidates(analysis: ScriptAnalysis, fileName: string, offset: number,
  catalog: IdentifierCatalog): ScriptCompletions {
  const empty = { items: [], isIncomplete: false };
  if (!analysisIsCurrent(analysis) || !samePolicyVersion(analysis.metadata, catalog)) return empty;
  const document = analysis.graph.modules.get(fileName);
  if (!document) return empty;
  const contexts = analysis.references.filter(reference => reference.literalRange?.fileName === fileName
    && reference.literalRange.start < offset && offset < reference.literalRange.end);
  if (!contexts.length || new Set(contexts.map(reference => reference.kind)).size !== 1) return empty;
  const reference = contexts[0], range = reference.literalRange!;
  const raw = document.text.slice(range.start, range.end);
  const prefix = document.text.slice(range.start + 1, offset);
  // Escape sequences do not have a one-to-one offset mapping; let TypeScript handle those strings.
  if (prefix.includes('\\')) return empty;
  const items: CompletionCandidate[] = [], seen = new Set<string>();
  const identifiers = reference.kind === 'pack-file'
    ? [...analysis.graph.packFiles.keys()].map(id => ({ id, kind: 'pack-file' as const })) : catalog.identifiers;
  for (const identifier of identifiers) {
    if (identifier.kind !== reference.kind || !identifier.id.startsWith(prefix) || seen.has(identifier.id)) continue;
    if (reference.slot.extensions && !reference.slot.extensions.some(extension => identifier.id.endsWith(extension))) continue;
    if (items.length === 200) return { items, isIncomplete: true };
    seen.add(identifier.id);
    items.push({ label: ('label' in identifier ? identifier.label : undefined) ?? identifier.id, id: identifier.id, kind: identifier.kind,
      insertText: JSON.stringify(identifier.id), range, oldText: raw, version: document.version });
  }
  return { items, isIncomplete: false };
}

export interface ReplacementQuickFix { readonly title: string; readonly plan: ContentEditPlan; }

/** Q17. A replacement is a source-owned, version-exact token mapping, never a fuzzy name suggestion. */
export function buildReplacementQuickFix(analysis: ScriptAnalysis, fileName: string, offset: number,
  currentVersions: PolicyVersions): ReplacementQuickFix | undefined {
  if (!analysisIsCurrent(analysis) || !analysis.graph.complete || analysis.metadata.catalogVersion === null
    || !samePolicyVersion(analysis.metadata, currentVersions)) return;
  const use = useAt(analysis, fileName, offset);
  if (use?.status !== 'known' || !use.symbolId || !use.binding.members.length) return;
  const document = analysis.graph.modules.get(fileName)!;
  const oldText = document.text.slice(use.range.start, use.range.end);
  const replacements = analysis.metadata.replacements.filter(replacement => replacement.symbolId === use.symbolId
    && replacement.oldText === oldText && replacement.provenance.authority === 'authoritative'
    && samePolicyVersion(replacement, currentVersions));
  if (replacements.length !== 1) return;
  const replacement = replacements[0];
  const scanner = ts.createScanner(ts.ScriptTarget.ES2022, false, ts.LanguageVariant.Standard, replacement.newText);
  const kind = scanner.scan(), newMember = scanner.getTokenValue();
  if (![ts.SyntaxKind.Identifier, ts.SyntaxKind.StringLiteral].includes(kind)
    || scanner.getTokenText() !== replacement.newText || scanner.scan() !== ts.SyntaxKind.EndOfFileToken) return;
  const oldScanner = ts.createScanner(ts.ScriptTarget.ES2022, false, ts.LanguageVariant.Standard, oldText);
  if (oldScanner.scan() !== kind) return;
  const target = analysis.metadata.symbols.find(symbol => symbol.symbolId === replacement.replacementSymbolId);
  const expectedBinding = { ...use.binding, members: [...use.binding.members.slice(0, -1), newMember] };
  if (!target || !authoritative(target.binding) || bindingKey(target.binding.value) !== bindingKey(expectedBinding)) return;
  const plan = scriptEditPlan(analysis, [editForRange(analysis, use.range, replacement.newText)]);
  return plan.safe ? { title: `Replace ${oldText} with ${replacement.newText}`, plan } : undefined;
}
