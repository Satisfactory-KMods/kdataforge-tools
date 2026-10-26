import * as ts from 'typescript';
import { AnalysisIssue, buildScriptGraph, createSnapshotProgram, GLOBAL_MODULE, isPackRelativePath, moduleEdge, PackFileAuthority,
  SDK_MODULE, ScriptGraph, ScriptGraphOptions, syntaxNodes } from './graph';
import { ApiBinding, ApiOperation, ApiPolicy, ApiPolicyMetadata, authoritative, bindingKey, PolicyFact,
  PolicyVersions, ReferenceKind, ReferenceSlot, samePolicyVersion, unknownPolicy } from './policy';

export interface SourceRange { readonly fileName: string; readonly start: number; readonly end: number; }
export interface ApiUse {
  readonly range: SourceRange;
  readonly expressionRange: SourceRange;
  readonly binding: ApiBinding;
  readonly symbolId?: string;
  readonly policy?: ApiPolicy;
  readonly status: 'known' | 'unknown' | 'ambiguous';
}
export interface ScriptReference {
  readonly kind: ReferenceKind;
  readonly range: SourceRange;
  readonly slot: ReferenceSlot;
  readonly symbolId: string;
  readonly status: 'static' | 'dynamic' | 'ambiguous' | 'invalid';
  readonly value?: string;
  /** Definition of an imported/local const, or the original literal. UTF-16 offsets include quotes. */
  readonly literalRange?: SourceRange;
  readonly pathAuthority?: PackFileAuthority;
  readonly reason?: string;
}
export interface ScriptExecutionContext {
  readonly authority?: string | number;
  readonly lifecycle?: string | number;
  readonly deterministicInitialization?: boolean;
}
export interface ScriptAnalysisOptions extends ScriptGraphOptions { readonly execution?: ScriptExecutionContext; }
export interface ScriptAnalysis {
  readonly graph: ScriptGraph;
  readonly metadata: ApiPolicyMetadata;
  readonly versions: PolicyVersions;
  readonly uses: readonly ApiUse[];
  readonly references: readonly ScriptReference[];
  readonly declaredCapabilities: ReadonlyMap<string, PolicyFact<readonly string[]>>;
  readonly issues: readonly AnalysisIssue[];
  readonly complete: boolean;
  /** Snapshot-only TypeScript program; exposed for symbol tooling, never emits or reads disk. */
  readonly program: ts.Program;
}

export function rangeOf(node: ts.Node): SourceRange {
  const file = node.getSourceFile();
  return { fileName: file.fileName, start: node.getStart(file), end: node.end };
}
export function unwrap(expression: ts.Expression): ts.Expression {
  while (ts.isParenthesizedExpression(expression) || ts.isAsExpression(expression)
    || ts.isTypeAssertionExpression(expression) || ts.isNonNullExpression(expression) || ts.isSatisfiesExpression(expression))
    expression = expression.expression;
  return expression;
}
function propertyName(name: ts.PropertyName): string | undefined {
  return ts.isIdentifier(name) || ts.isStringLiteralLike(name) ? name.text : undefined;
}
function isConst(declaration: ts.VariableDeclaration): boolean {
  return ts.isVariableDeclarationList(declaration.parent) && !!(declaration.parent.flags & ts.NodeFlags.Const);
}

type Origin = Omit<ApiBinding, 'operation'>;
class SymbolResolver {
  readonly checker: ts.TypeChecker;
  private readonly globals = new Set<string>();
  constructor(readonly program: ts.Program, readonly graph: ScriptGraph, metadata: ApiPolicyMetadata) {
    this.checker = program.getTypeChecker();
    for (const symbol of metadata.symbols) if (authoritative(symbol.binding) && symbol.binding.value.kind === 'global'
      && symbol.binding.value.module === GLOBAL_MODULE) this.globals.add(symbol.binding.value.exportName);
  }

  private importOrigin(declaration: ts.ImportSpecifier | ts.ImportClause | ts.NamespaceImport
    | ts.ExportSpecifier, seen: Set<ts.Symbol>): Origin | undefined {
    let owner: ts.Node = declaration;
    while (!ts.isImportDeclaration(owner) && !ts.isExportDeclaration(owner) && owner.parent) owner = owner.parent;
    if ((!ts.isImportDeclaration(owner) && !ts.isExportDeclaration(owner))
      || !owner.moduleSpecifier || !ts.isStringLiteral(owner.moduleSpecifier)) return;
    const module = owner.moduleSpecifier.text;
    const exportName = ts.isImportSpecifier(declaration) || ts.isExportSpecifier(declaration)
      ? (declaration.propertyName ?? declaration.name).text : ts.isImportClause(declaration) ? 'default' : '*';
    const edge = moduleEdge(this.graph, owner.getSourceFile().fileName, module);
    if (edge?.external) return { kind: 'export', module, exportName, members: [] };
    if (!edge?.to || exportName === '*') return;
    const sourceFile = this.program.getSourceFile(edge.to);
    const symbol = sourceFile && this.checker.getSymbolAtLocation(sourceFile);
    const exported = symbol && this.checker.getExportsOfModule(symbol).find(value => value.name === exportName);
    return exported && this.symbolOrigin(exported, seen);
  }

  private symbolOrigin(symbol: ts.Symbol, seen: Set<ts.Symbol>): Origin | undefined {
    if (seen.has(symbol) || seen.size >= 16) return;
    seen = new Set(seen).add(symbol);
    const declarations = symbol.declarations ?? [];
    if (declarations.length !== 1) return;
    const declaration = declarations[0];
    if (ts.isImportSpecifier(declaration) || ts.isImportClause(declaration) || ts.isNamespaceImport(declaration)
      || ts.isExportSpecifier(declaration)) {
      const imported = this.importOrigin(declaration, seen);
      if (imported) return imported;
      if (symbol.flags & ts.SymbolFlags.Alias) return this.symbolOrigin(this.checker.getAliasedSymbol(symbol), seen);
    }
    if (ts.isVariableDeclaration(declaration) && isConst(declaration) && declaration.initializer && ts.isIdentifier(declaration.name))
      return this.origin(declaration.initializer, seen);
    if (ts.isParameter(declaration)) {
      const fn = declaration.parent;
      if ((ts.isArrowFunction(fn) || ts.isFunctionExpression(fn)) && fn.parameters[0] === declaration
        && ts.isCallExpression(fn.parent) && fn.parent.arguments[0] === fn) {
        const factory = this.origin(fn.parent.expression, seen);
        if (factory?.module === SDK_MODULE && factory.exportName === 'defineHandler' && !factory.members.length)
          return { kind: 'context', module: SDK_MODULE, exportName: 'Context', members: [] };
      }
      if (declaration.type && ts.isTypeReferenceNode(declaration.type) && ts.isIdentifier(declaration.type.typeName)) {
        const type = this.checker.getSymbolAtLocation(declaration.type.typeName);
        const origin = type && this.symbolOrigin(type, seen);
        if (origin?.module === SDK_MODULE && origin.exportName === 'Context' && !origin.members.length)
          return { ...origin, kind: 'context' };
      }
    }
    return;
  }

  private isPackNamespace(expression: ts.Expression, seen = new Set<ts.Symbol>()): boolean {
    expression = unwrap(expression);
    if (!ts.isIdentifier(expression)) return false;
    const symbol = this.checker.getSymbolAtLocation(expression);
    if (!symbol || seen.has(symbol) || seen.size >= 16 || symbol.declarations?.length !== 1) return false;
    const declaration = symbol.declarations[0];
    if (ts.isNamespaceImport(declaration)) {
      const owner = declaration.parent.parent;
      if (!ts.isImportDeclaration(owner) || !ts.isStringLiteral(owner.moduleSpecifier)) return false;
      const edge = moduleEdge(this.graph, owner.getSourceFile().fileName, owner.moduleSpecifier.text);
      return !!edge?.to && this.graph.modules.has(edge.to);
    }
    return ts.isVariableDeclaration(declaration) && isConst(declaration) && !!declaration.initializer
      && this.isPackNamespace(declaration.initializer, new Set(seen).add(symbol));
  }

  origin(expression: ts.Expression, seen = new Set<ts.Symbol>()): Origin | undefined {
    expression = unwrap(expression);
    if (ts.isIdentifier(expression)) {
      const symbol = this.checker.getSymbolAtLocation(expression);
      if (symbol) return this.symbolOrigin(symbol, seen);
      if (this.globals.has(expression.text)) return { kind: 'global', module: GLOBAL_MODULE, exportName: expression.text, members: [] };
      return;
    }
    if (ts.isNewExpression(expression)) {
      const constructor = this.origin(expression.expression, seen);
      return constructor?.kind === 'global' ? { ...constructor, kind: 'instance' } : undefined;
    }
    if (ts.isPropertyAccessExpression(expression) || ts.isElementAccessExpression(expression)) {
      const name = ts.isPropertyAccessExpression(expression) ? expression.name.text
        : expression.argumentExpression && ts.isStringLiteralLike(expression.argumentExpression) ? expression.argumentExpression.text : undefined;
      if (name === undefined) return;
      const base = this.origin(expression.expression, seen);
      if (base) return base.exportName === '*' ? { ...base, exportName: name }
        : { ...base, members: [...base.members, name] };
      if (!this.isPackNamespace(expression.expression)) return;
      const symbol = this.checker.getSymbolAtLocation(ts.isPropertyAccessExpression(expression) ? expression.name : expression);
      return symbol && this.symbolOrigin(symbol, seen);
    }
    return;
  }

  literal(expression: ts.Expression, seen = new Set<ts.Symbol>()): Pick<ScriptReference, 'status' | 'value' | 'literalRange'> {
    expression = unwrap(expression);
    if (ts.isStringLiteralLike(expression)) return { status: 'static', value: expression.text, literalRange: rangeOf(expression) };
    if (ts.isIdentifier(expression) || ts.isPropertyAccessExpression(expression)) {
      if (ts.isPropertyAccessExpression(expression) && !this.isPackNamespace(expression.expression)) return { status: 'dynamic' };
      let symbol = this.checker.getSymbolAtLocation(ts.isPropertyAccessExpression(expression) ? expression.name : expression);
      if (!symbol) return { status: 'dynamic' };
      const imported = !!(symbol.flags & ts.SymbolFlags.Alias);
      if (symbol.flags & ts.SymbolFlags.Alias) symbol = this.checker.getAliasedSymbol(symbol);
      if (seen.has(symbol) || seen.size >= 16 || symbol.declarations?.length !== 1) return { status: 'ambiguous' };
      const declaration = symbol.declarations[0];
      if (ts.isIdentifier(expression) && !imported && declaration.getSourceFile() !== expression.getSourceFile())
        return { status: 'dynamic' };
      if (ts.isVariableDeclaration(declaration) && isConst(declaration) && declaration.initializer && ts.isIdentifier(declaration.name))
        return this.literal(declaration.initializer, new Set(seen).add(symbol));
    }
    return { status: 'dynamic' };
  }
}

function referenceExpression(expression: ts.Expression, slot: ReferenceSlot): ts.Expression | undefined {
  for (const name of slot.path) {
    expression = unwrap(expression);
    if (!ts.isObjectLiteralExpression(expression)
      || expression.properties.some(property => !ts.isPropertyAssignment(property) || propertyName(property.name) === undefined)) return;
    const matches = expression.properties.filter(property => propertyName(property.name!) === name);
    if (matches.length !== 1 || !ts.isPropertyAssignment(matches[0])) return;
    expression = matches[0].initializer;
  }
  return expression;
}

function operationOf(node: ts.Expression): ApiOperation {
  const parent = node.parent;
  if (ts.isNewExpression(parent) && parent.expression === node) return 'construct';
  if (ts.isCallExpression(parent) && parent.expression === node) return 'call';
  if ((ts.isBinaryExpression(parent) && parent.left === node && parent.operatorToken.kind >= ts.SyntaxKind.FirstAssignment
    && parent.operatorToken.kind <= ts.SyntaxKind.LastAssignment)
    || ts.isDeleteExpression(parent) || ts.isPostfixUnaryExpression(parent)
    || (ts.isPrefixUnaryExpression(parent) && [ts.SyntaxKind.PlusPlusToken, ts.SyntaxKind.MinusMinusToken].includes(parent.operator)))
    return 'write';
  return 'read';
}

function metaCapabilities(file: ts.SourceFile, resolver: SymbolResolver): PolicyFact<readonly string[]> {
  const exported = file.statements.filter(statement => ts.isVariableStatement(statement)
    && statement.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.ExportKeyword)) as ts.VariableStatement[];
  const declarations = exported.flatMap(statement => [...statement.declarationList.declarations])
    .filter(declaration => ts.isIdentifier(declaration.name) && declaration.name.text === 'meta');
  const exportedAlias = file.statements.some(statement => ts.isExportDeclaration(statement) && statement.exportClause
    && ts.isNamedExports(statement.exportClause) && statement.exportClause.elements.some(element => element.name.text === 'meta'));
  if (exportedAlias || declarations.length > 1) return unknownPolicy('Exported meta is ambiguous');
  const known = (value: readonly string[]): PolicyFact<readonly string[]> => ({ status: 'known', value,
    provenance: { source: file.fileName, version: String(resolver.graph.modules.get(file.fileName)!.version), authority: 'authoritative' } });
  if (!declarations.length) return known([]);
  const declaration = declarations[0];
  const initializer = declaration.initializer && unwrap(declaration.initializer);
  if (!isConst(declaration) || !initializer || !ts.isCallExpression(initializer)) return unknownPolicy('Meta is not a static defineMeta call');
  const origin = resolver.origin(initializer.expression);
  if (origin?.module !== SDK_MODULE || origin.exportName !== 'defineMeta' || origin.members.length || initializer.arguments.length !== 1)
    return unknownPolicy('Meta does not resolve to SDK defineMeta');
  const object = unwrap(initializer.arguments[0]);
  if (!ts.isObjectLiteralExpression(object) || object.properties.length !== 1) return unknownPolicy('Meta is not a static object');
  const property = object.properties[0];
  if (!ts.isPropertyAssignment(property) || propertyName(property.name) !== 'capabilities')
    return unknownPolicy('Capabilities contain a computed property, getter or spread');
  const array = unwrap(property.initializer);
  if (!ts.isArrayLiteralExpression(array) || !array.elements.every(ts.isStringLiteral)) return unknownPolicy('Capabilities are not literal strings');
  const values = array.elements.map(value => (value as ts.StringLiteral).text);
  if (new Set(values).size !== values.length) return unknownPolicy('Duplicate declared capability');
  return known(values);
}

export function analysisIsCurrent(analysis: ScriptAnalysis): boolean {
  return analysis.graph.ticket.isCurrent() && samePolicyVersion(analysis.metadata, analysis.versions);
}

export function analyzeScriptGraph(graph: ScriptGraph, metadata: ApiPolicyMetadata,
  versions: PolicyVersions = metadata, execution: ScriptExecutionContext = {}): ScriptAnalysis {
  const program = createSnapshotProgram(graph);
  const uses: ApiUse[] = [], references: ScriptReference[] = [], issues: AnalysisIssue[] = [];
  const declaredCapabilities = new Map<string, PolicyFact<readonly string[]>>();
  const bindings = new Map<string, ApiPolicy[]>();
  for (const symbol of metadata.symbols) if (authoritative(symbol.binding)) {
    const key = bindingKey(symbol.binding.value);
    bindings.set(key, [...(bindings.get(key) ?? []), symbol]);
  }
  const resolver = new SymbolResolver(program, graph, metadata);
  const addIssue = (code: string, node: ts.Node) => { const range = rangeOf(node); issues.push({ code, fileName: range.fileName, start: range.start }); };
  if (!samePolicyVersion(metadata, versions)) issues.push({ code: 'policy-version-mismatch' });
  for (const module of graph.modules.values()) {
    if (!graph.ticket.isCurrent()) { issues.push({ code: 'stale-generation' }); break; }
    declaredCapabilities.set(module.fileName, metaCapabilities(module.sourceFile, resolver));
    const nodes = syntaxNodes(module.sourceFile);
    const lexicalNames = new Map<ts.Node, Set<string>>();
    for (const node of nodes) {
      if (!ts.isVariableDeclaration(node) || !ts.isIdentifier(node.name)) continue;
      let scope: ts.Node = node.parent;
      while (scope.parent && !ts.isBlock(scope) && !ts.isSourceFile(scope) && !ts.isForStatement(scope)
        && !ts.isForOfStatement(scope) && !ts.isForInStatement(scope) && !ts.isCaseBlock(scope)) scope = scope.parent;
      const names = lexicalNames.get(scope) ?? new Set<string>();
      if (names.has(node.name.text)) addIssue('ambiguous-binding', node);
      names.add(node.name.text);
      lexicalNames.set(scope, names);
    }
    for (const node of nodes) {
      if (!graph.ticket.isCurrent()) { addIssue('stale-generation', node); break; }
      if (ts.isElementAccessExpression(node) && resolver.origin(node.expression)
        && !ts.isStringLiteralLike(node.argumentExpression)) addIssue('dynamic-api-access', node);
      if (ts.isVariableDeclaration(node) && !ts.isIdentifier(node.name) && node.initializer && resolver.origin(node.initializer))
        addIssue('unsupported-api-destructuring', node);
      if (ts.isNewExpression(node) && resolver.origin(node)?.kind === 'instance'
        && !ts.isPropertyAccessExpression(node.parent) && !ts.isElementAccessExpression(node.parent)
        && !ts.isExpressionStatement(node.parent)
        && !(ts.isVariableDeclaration(node.parent) && isConst(node.parent) && ts.isIdentifier(node.parent.name)))
        addIssue('api-instance-escape', node);
      if ((ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node))
        && ts.isCallExpression(unwrap(node.expression))
        && resolver.origin((unwrap(node.expression) as ts.CallExpression).expression)) addIssue('unresolved-api-result', node);
      if (!ts.isIdentifier(node) && !ts.isPropertyAccessExpression(node) && !ts.isElementAccessExpression(node)) continue;
      const parent = node.parent;
      if ((ts.isPropertyAccessExpression(parent) && (parent.expression === node || parent.name === node))
        || (ts.isElementAccessExpression(parent) && parent.expression === node)) continue;
      if (ts.isIdentifier(node) && ((ts.isParameter(parent) || ts.isVariableDeclaration(parent)) && parent.name === node
        || ts.isImportSpecifier(parent) || ts.isImportClause(parent) || ts.isNamespaceImport(parent)
        || ts.isExportSpecifier(parent) || ts.isTypeReferenceNode(parent) || ts.isPropertyAssignment(parent) && parent.name === node)) continue;
      const origin = resolver.origin(node);
      if (!origin) continue;
      if (!origin.members.length && ts.isVariableDeclaration(parent) && isConst(parent) && ts.isIdentifier(parent.name)) continue;
      if (origin.kind === 'context' && !origin.members.length) {
        if (!(ts.isVariableDeclaration(parent) && isConst(parent) && ts.isIdentifier(parent.name))) addIssue('api-object-escape', node);
        continue;
      }
      const binding: ApiBinding = { ...origin, operation: operationOf(node) };
      const policies = bindings.get(bindingKey(binding)) ?? [];
      const policy = policies.length === 1 ? policies[0] : undefined;
      const terminal = ts.isPropertyAccessExpression(node) ? node.name : ts.isElementAccessExpression(node) ? node.argumentExpression : node;
      const use: ApiUse = { range: rangeOf(terminal), expressionRange: rangeOf(node), binding, policy,
        symbolId: policy?.symbolId, status: policy ? 'known' : policies.length ? 'ambiguous' : 'unknown' };
      uses.push(use);
      if (!policy) addIssue(policies.length ? 'ambiguous-api' : 'unknown-api-policy', node);
      if (policy && ['call', 'construct', 'write'].includes(binding.operation)) {
        for (const field of ['authority', 'lifecycle'] as const) if (authoritative(policy[field])) {
          if (execution[field] === undefined) addIssue(`unknown-execution-${field}`, node);
          else if (execution[field] !== policy[field].value) addIssue(`${field}-mismatch`, node);
        }
        if (authoritative(policy.deterministicInitialization) && !policy.deterministicInitialization.value) {
          if (execution.deterministicInitialization === undefined) addIssue('unknown-initialization-mode', node);
          else if (execution.deterministicInitialization) addIssue('deterministic-initialization-forbidden', node);
        }
      }
      if ((!ts.isCallExpression(parent) && !ts.isNewExpression(parent)) || parent.expression !== node) continue;
      if (!policy || !authoritative(policy.references)) { addIssue('unknown-reference-policy', node); continue; }
      for (const slot of policy.references.value) {
        const argument = parent.arguments?.[slot.argument];
        if (!argument) { addIssue('missing-reference-argument', parent); continue; }
        const expression = referenceExpression(argument, slot);
        let reference: ScriptReference = { kind: slot.kind, range: rangeOf(expression ?? argument), slot,
          symbolId: policy.symbolId, ...(expression ? resolver.literal(expression) : { status: 'dynamic' as const }) };
        if (slot.kind === 'pack-file' && (!expression || !ts.isStringLiteralLike(unwrap(expression))))
          reference = { ...reference, status: 'dynamic', pathAuthority: undefined, reason: 'pack-path-must-be-literal' };
        if (slot.kind === 'pack-file' && reference.status === 'static') {
          const value = reference.value!;
          const authority = graph.packFiles.get(value);
          const validExtension = !slot.extensions || slot.extensions.some(extension => value.endsWith(extension));
          reference = isPackRelativePath(value) && authority && validExtension
            ? { ...reference, pathAuthority: authority }
            : { ...reference, status: 'invalid', reason: !isPackRelativePath(value) ? 'pack-path-escape'
              : !validExtension ? 'pack-file-type-mismatch' : 'undeclared-pack-file' };
        }
        references.push(reference);
        if (reference.status !== 'static') addIssue(`${reference.status}-reference`, expression ?? argument);
      }
    }
  }
  return { graph, metadata, versions, program, uses, references, declaredCapabilities, issues,
    complete: graph.complete && !issues.length && graph.ticket.isCurrent() };
}

export async function analyzeScripts(options: ScriptAnalysisOptions, metadata: ApiPolicyMetadata,
  versions: PolicyVersions = metadata): Promise<ScriptAnalysis> {
  return analyzeScriptGraph(await buildScriptGraph(options), metadata, versions, options.execution);
}
