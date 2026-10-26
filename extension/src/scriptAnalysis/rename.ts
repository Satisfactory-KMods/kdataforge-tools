import * as ts from 'typescript';
import { ContentEditPlan, ContentTextEdit, createContentEditPlan } from '../contentRename';
import { analysisIsCurrent, ScriptAnalysis, SourceRange } from './analyzer';
import { ReferenceKind } from './policy';
import { moduleEdge, syntaxNodes } from './graph';

export function editForRange(analysis: ScriptAnalysis, range: SourceRange, newText: string): ContentTextEdit {
  const document = analysis.graph.modules.get(range.fileName);
  if (!document) throw new Error('Edit is outside the snapshot graph');
  return { ...range, oldText: document.text.slice(range.start, range.end), newText, version: document.version };
}

export function scriptEditPlan(analysis: ScriptAnalysis, edits: readonly ContentTextEdit[],
  blockers: readonly string[] = []): ContentEditPlan {
  const reasons = [...blockers];
  if (!analysisIsCurrent(analysis)) reasons.push('stale-analysis');
  return createContentEditPlan(analysis.graph.ticket.generation, [...analysis.graph.modules.values()], edits, reasons);
}

function renameBlockers(analysis: ScriptAnalysis): string[] {
  const blockers: string[] = [];
  if (!analysis.complete) blockers.push('incomplete-analysis', ...analysis.graph.issues.map(issue => issue.code),
    ...analysis.issues.map(issue => issue.code));
  if (!analysis.graph.allPackScriptsProvided) blockers.push('pack-scope-not-proven');
  return blockers;
}

export interface ContentRenameRequest {
  readonly kind: Exclude<ReferenceKind, 'pack-file'>;
  readonly oldId: string;
  readonly newId: string;
  /** Optional complete ID inventory from the YAML/catalog provider, used for collision refusal. */
  readonly existingIds?: readonly string[];
}

/** Imported const strings are edited at their declaration, once. Expressions and getters are never evaluated. */
export function planContentRename(analysis: ScriptAnalysis, request: ContentRenameRequest): ContentEditPlan {
  const blockers = renameBlockers(analysis);
  const edits: ContentTextEdit[] = [];
  if (!['asset', 'class', 'content'].includes(request.kind)) blockers.push('unsupported-rename-kind');
  if (!request.oldId || !request.newId || /[\x00\r\n]/.test(request.newId) || request.oldId === request.newId)
    blockers.push('invalid-rename');
  if (request.existingIds?.includes(request.newId)
    || analysis.references.some(reference => reference.kind === request.kind && reference.value === request.newId))
    blockers.push('content-id-collision');
  const roles = new Map<string, Set<ReferenceKind>>();
  for (const reference of analysis.references) if (reference.literalRange) {
    const key = JSON.stringify([reference.literalRange.fileName, reference.literalRange.start]);
    const kinds = roles.get(key) ?? new Set<ReferenceKind>();
    kinds.add(reference.kind);
    roles.set(key, kinds);
  }
  for (const reference of analysis.references) {
    if (reference.status !== 'static') { blockers.push('dynamic-or-ambiguous-reference'); continue; }
    if (reference.kind !== request.kind || reference.value !== request.oldId || !reference.literalRange) continue;
    const range = reference.literalRange;
    if (roles.get(JSON.stringify([range.fileName, range.start]))!.size > 1) { blockers.push('ambiguous-literal-role'); continue; }
    edits.push(editForRange(analysis, range, JSON.stringify(request.newId)));
  }
  return scriptEditPlan(analysis, edits, blockers);
}

/** Rename local/exported TS symbols, following imports and preserving TypeScript's alias/shorthand edits. */
export function planTypeScriptSymbolRename(analysis: ScriptAnalysis, fileName: string, position: number,
  newName: string): ContentEditPlan {
  const blockers = renameBlockers(analysis);
  const scanner = ts.createScanner(ts.ScriptTarget.ES2022, false, ts.LanguageVariant.Standard, newName);
  if (scanner.scan() !== ts.SyntaxKind.Identifier || scanner.getTokenText() !== newName
    || scanner.scan() !== ts.SyntaxKind.EndOfFileToken) blockers.push('invalid-symbol-name');
  if (!analysis.graph.modules.has(fileName) || !Number.isSafeInteger(position) || position < 0
    || position >= (analysis.graph.modules.get(fileName)?.text.length ?? 0)) blockers.push('unknown-symbol-location');
  if (blockers.length || !analysisIsCurrent(analysis)) return scriptEditPlan(analysis, [], blockers);
  const sourceFile = analysis.graph.modules.get(fileName)!.sourceFile;
  const target = syntaxNodes(sourceFile).find(node => ts.isIdentifier(node)
    && node.getStart(sourceFile) <= position && position <= node.end);
  const symbol = target && analysis.program.getTypeChecker().getSymbolAtLocation(target);
  const declaration = symbol?.declarations?.length === 1 ? symbol.declarations[0] : undefined;
  // Property names can have computed/string references that TypeScript's lexical rename cannot prove.
  const lexical = declaration && (ts.isImportSpecifier(declaration) || ts.isImportClause(declaration)
    || ts.isNamespaceImport(declaration) || ts.isFunctionDeclaration(declaration) || ts.isClassDeclaration(declaration)
    || (ts.isVariableDeclaration(declaration) && ts.isIdentifier(declaration.name))
    || (ts.isParameter(declaration) && ts.isIdentifier(declaration.name) && !declaration.modifiers?.length));
  if (!lexical) return scriptEditPlan(analysis, [], ['symbol-rename-unproven']);
  // A conservative collision check avoids introducing capture in any importing scope.
  for (const module of analysis.graph.modules.values()) for (const node of syntaxNodes(module.sourceFile)) {
    if (ts.isIdentifier(node) && node.text === newName) blockers.push('symbol-name-collision');
  }
  if (blockers.length) return scriptEditPlan(analysis, [], blockers);
  const graph = analysis.graph;
  const service = ts.createLanguageService({
    getCompilationSettings: () => analysis.program.getCompilerOptions(),
    getScriptFileNames: () => [...graph.modules.keys()],
    getScriptVersion: name => String(graph.modules.get(name)?.version ?? ''),
    getScriptSnapshot: name => {
      const document = graph.modules.get(name);
      return document && ts.ScriptSnapshot.fromString(document.text);
    },
    getCurrentDirectory: () => graph.root, getDefaultLibFileName: () => '',
    fileExists: name => graph.modules.has(name), readFile: name => graph.modules.get(name)?.text,
    useCaseSensitiveFileNames: () => true,
    resolveModuleNames: (names, containingFile) => names.map(name => {
      const to = moduleEdge(graph, containingFile, name)?.to;
      return to && graph.modules.has(to) ? { resolvedFileName: to } : undefined;
    }),
    getCancellationToken: () => ({ isCancellationRequested: () => !graph.ticket.isCurrent(),
      throwIfCancellationRequested: () => { if (!graph.ticket.isCurrent()) throw new ts.OperationCanceledException(); } }),
  });
  try {
    const info = service.getRenameInfo(fileName, position, { allowRenameOfImportPath: false });
    if (!info.canRename) return scriptEditPlan(analysis, [], ['unresolved-symbol']);
    const locations = service.findRenameLocations(fileName, position, false, false, true);
    if (!locations?.length) return scriptEditPlan(analysis, [], ['unresolved-symbol']);
    const edits = locations.map(location => editForRange(analysis, { fileName: location.fileName,
      start: location.textSpan.start, end: location.textSpan.start + location.textSpan.length },
    `${location.prefixText ?? ''}${newName}${location.suffixText ?? ''}`));
    return scriptEditPlan(analysis, edits);
  } catch {
    return scriptEditPlan(analysis, [], ['symbol-rename-unproven']);
  } finally { service.dispose(); }
}
