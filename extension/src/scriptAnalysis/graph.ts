import * as fs from 'node:fs/promises';
import { constants } from 'node:fs';
import * as path from 'node:path';
import { createHash } from 'node:crypto';
import * as ts from 'typescript';
import type { Provenance } from './policy';

export const SCRIPT_LIMITS = Object.freeze({ modules: 64, bytes: 4 * 1024 * 1024, fileBytes: 512 * 1024,
  depth: 16, resolutions: 512, nodes: 200_000, syntaxDepth: 256 });
export const SDK_MODULE = '@sf-kmods/kdataforgejs';
export const SDK_MODULES: readonly string[] = Object.freeze([SDK_MODULE, 'satisfactory']);
export const GLOBAL_MODULE = 'global';

export interface DeclaredPackFile { readonly path: string; readonly provenance: Provenance; }
export interface PackFileAuthority extends DeclaredPackFile { readonly fileName: string; }

/** A literal pack-relative path, with no URLs, encoding tricks, drive names or traversal segments. */
export function isPackRelativePath(value: string): boolean {
  return !!value && !path.posix.isAbsolute(value) && !path.win32.isAbsolute(value)
    && !/[\\%\x00-\x1f:#?]/.test(value) && value.split('/').every(part => part !== '' && part !== '.' && part !== '..');
}

export interface DocumentSnapshot {
  readonly fileName: string;
  readonly text: string;
  /** Editor version for open buffers; a content hash is suitable for closed files. */
  readonly version: string | number;
}

export interface AnalysisTicket {
  readonly generation: number;
  readonly signal?: AbortSignal;
  isCurrent(): boolean;
}

/** Start a new generation on every buffer, pack, SDK or catalog change. */
export class AnalysisGeneration {
  private generation = 0;
  private cancellation?: AbortController;
  next(signal?: AbortSignal): AnalysisTicket {
    this.cancellation?.abort();
    const cancellation = new AbortController();
    this.cancellation = cancellation;
    const generation = ++this.generation;
    if (signal?.aborted) cancellation.abort();
    else if (signal) {
      const abort = () => cancellation.abort();
      signal.addEventListener('abort', abort, { once: true });
      cancellation.signal.addEventListener('abort', () => signal.removeEventListener('abort', abort), { once: true });
    }
    return { generation, signal: cancellation.signal,
      isCurrent: () => generation === this.generation && !cancellation.signal.aborted };
  }
  invalidate(): void { this.generation++; this.cancellation?.abort(); }
}

async function cancellable<T>(ticket: AnalysisTicket, work: () => Promise<T>): Promise<T> {
  if (!ticket.isCurrent()) throw new Error('stale-generation');
  const signal = ticket.signal;
  if (!signal) return work();
  let abort: () => void = () => {};
  const cancelled = new Promise<never>((_, reject) => {
    abort = () => reject(new Error('stale-generation'));
    signal.addEventListener('abort', abort, { once: true });
  });
  try { return await Promise.race([work(), cancelled]); }
  finally { signal.removeEventListener('abort', abort); }
}

/** Trusted host adapter. Never implement this by loading a pack's JS or tsconfig. */
export interface ScriptSourceHost {
  realpath(fileName: string): Promise<string>;
  read(fileName: string, maxBytes: number): Promise<DocumentSnapshot>;
}

export function createFileSystemSourceHost(): ScriptSourceHost {
  return {
    realpath: fs.realpath,
    async read(fileName, maxBytes) {
      const handle = await fs.open(fileName, constants.O_RDONLY | constants.O_NOFOLLOW);
      try {
        const before = await handle.stat();
        if (!before.isFile() || before.size > maxBytes) throw new Error('file-limit');
        const bytes = Buffer.alloc(maxBytes + 1);
        let length = 0;
        while (length < bytes.length) {
          const result = await handle.read(bytes, length, bytes.length - length, length);
          if (!result.bytesRead) break;
          length += result.bytesRead;
        }
        const after = await handle.stat();
        if (length > maxBytes) throw new Error('file-limit');
        if (before.size !== after.size || before.mtimeMs !== after.mtimeMs || before.ctimeMs !== after.ctimeMs)
          throw new Error('changed-during-read');
        const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes.subarray(0, length));
        return { fileName, text, version: createHash('sha256').update(bytes.subarray(0, length)).digest('hex') };
      } finally { await handle.close(); }
    },
  };
}

export interface AnalysisIssue { readonly code: string; readonly fileName?: string; readonly start?: number; }
export interface ScriptModule extends DocumentSnapshot { readonly sourceFile: ts.SourceFile; readonly depth: number; }
export interface ModuleEdge {
  readonly from: string;
  readonly specifier: string;
  readonly to?: string;
  readonly external?: boolean;
}
export interface ScriptGraph {
  readonly root: string;
  readonly entrypoints: readonly string[];
  readonly modules: ReadonlyMap<string, ScriptModule>;
  readonly edges: readonly ModuleEdge[];
  readonly issues: readonly AnalysisIssue[];
  readonly complete: boolean;
  /** Caller attests that entrypoints include every script in the pack, including disconnected files. */
  readonly allPackScriptsProvided: boolean;
  /** UTF-8 source bytes consumed, including files rejected during parsing. */
  readonly bytes: number;
  readonly ticket: AnalysisTicket;
  readonly packFiles: ReadonlyMap<string, PackFileAuthority>;
}
export interface ScriptGraphOptions {
  readonly packRoot: string;
  readonly entrypoints: readonly string[];
  readonly host?: ScriptSourceHost;
  readonly ticket?: AnalysisTicket;
  readonly allPackScriptsProvided?: boolean;
  /** Authoritative declared file inventory; paths are canonicalized without reading file contents. */
  readonly declaredPackFiles?: readonly DeclaredPackFile[];
}

const moduleIndexes = new WeakMap<ScriptGraph, Map<string, ModuleEdge>>();
export function moduleEdge(graph: ScriptGraph, from: string, specifier: string): ModuleEdge | undefined {
  let index = moduleIndexes.get(graph);
  if (!index) {
    index = new Map(graph.edges.map(edge => [JSON.stringify([edge.from, edge.specifier]), edge]));
    moduleIndexes.set(graph, index);
  }
  return index.get(JSON.stringify([from, specifier]));
}

function pathOperations(root: string): typeof path.posix {
  if (/^(?:[A-Za-z]:[\\/]|\\\\)/u.test(root)) return path.win32;
  if (root.startsWith('/')) return path.posix;
  return path;
}

function contained(paths: typeof path.posix, root: string, fileName: string): boolean {
  const relative = paths.relative(root, fileName);
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${paths.sep}`) && !paths.isAbsolute(relative)
    && !relative.split(paths.sep).includes('node_modules');
}

/** Iterative traversal also bounds deeply nested hostile syntax before checker use. */
export function syntaxNodes(sourceFile: ts.SourceFile): ts.Node[] {
  const pending = [{ node: sourceFile as ts.Node, depth: 0 }];
  const result: ts.Node[] = [];
  while (pending.length) {
    const { node, depth } = pending.pop()!;
    if (depth > SCRIPT_LIMITS.syntaxDepth || result.length >= SCRIPT_LIMITS.nodes) throw new Error('syntax-limit');
    result.push(node);
    ts.forEachChild(node, child => { pending.push({ node: child, depth: depth + 1 }); });
  }
  return result;
}

function candidates(paths: typeof path.posix, from: string, specifier: string): string[] {
  const base = paths.resolve(paths.dirname(from), specifier);
  if (/\.[cm]?tsx?$/.test(base)) return [base];
  if (/\.js$/.test(base)) return [base.slice(0, -3) + '.ts', base.slice(0, -3) + '.tsx'];
  if (/\.[cm]js$/.test(base)) return [base.slice(0, -2) + 'ts'];
  if (paths.extname(base)) return [];
  return [base + '.ts', base + '.tsx', paths.join(base, 'index.ts'), paths.join(base, 'index.tsx')];
}

export async function buildScriptGraph(options: ScriptGraphOptions): Promise<ScriptGraph> {
  const host = options.host ?? createFileSystemSourceHost();
  const ticket = options.ticket ?? new AnalysisGeneration().next();
  const modules = new Map<string, ScriptModule>();
  const edges: ModuleEdge[] = [];
  const issues: AnalysisIssue[] = [];
  const entrypoints: string[] = [];
  const packFiles = new Map<string, PackFileAuthority>();
  const paths = pathOperations(options.packRoot);
  let root = paths.resolve(options.packRoot), bytes = 0, resolutions = 0, nodes = 0;
  const issue = (code: string, fileName?: string, start?: number) => { issues.push({ code, fileName, start }); };
  const result = (): ScriptGraph => ({ root, entrypoints, modules, edges, issues,
    complete: !issues.length && ticket.isCurrent(), allPackScriptsProvided: options.allPackScriptsProvided === true,
    bytes, ticket, packFiles });
  const current = () => {
    if (ticket.isCurrent()) return true;
    if (!issues.some(value => value.code === 'stale-generation')) issue('stale-generation');
    return false;
  };
  if (!current()) return result();
  try { root = paths.resolve(await cancellable(ticket, () => host.realpath(root))); }
  catch { issue('unreadable-root'); return result(); }
  const queue: { fileName: string; depth: number }[] = [];
  const scheduled = new Set<string>();
  const resolve = async (fileNames: readonly string[]): Promise<string | undefined> => {
    const matches = new Set<string>();
    for (const fileName of fileNames) {
      if (!current()) return;
      if (++resolutions > SCRIPT_LIMITS.resolutions) { issue('resolution-limit'); return; }
      if (!contained(paths, root, fileName)) { issue('import-escape', fileName); return; }
      try {
        const canonical = paths.resolve(await cancellable(ticket, () => host.realpath(fileName)));
        if (!current()) return;
        if (!contained(paths, root, canonical)) { issue('import-escape', fileName); return; }
        matches.add(canonical);
      } catch { /* Missing candidates are expected; no package or directory configuration is read. */ }
    }
    if (matches.size > 1) { issue('ambiguous-import', fileNames[0]); return; }
    return matches.values().next().value;
  };
  const schedule = (fileName: string, depth: number) => {
    if (scheduled.has(fileName)) return;
    if (depth > SCRIPT_LIMITS.depth) { issue('depth-limit', fileName); return; }
    if (scheduled.size >= SCRIPT_LIMITS.modules) { issue('module-limit', fileName); return; }
    scheduled.add(fileName);
    queue.push({ fileName, depth });
  };
  if ((options.declaredPackFiles?.length ?? 0) > SCRIPT_LIMITS.resolutions) issue('pack-file-limit');
  for (const declaration of (options.declaredPackFiles ?? []).slice(0, SCRIPT_LIMITS.resolutions)) {
    if (!current()) return result();
    if (!isPackRelativePath(declaration.path) || declaration.provenance.authority !== 'authoritative'
      || !declaration.provenance.source || !declaration.provenance.version) { issue('invalid-pack-file-declaration'); continue; }
    if (packFiles.has(declaration.path)) { issue('ambiguous-pack-file-declaration'); continue; }
    const fileName = await resolve([paths.resolve(root, declaration.path)]);
    if (fileName) packFiles.set(declaration.path, { ...declaration, fileName });
    else issue('unresolved-pack-file', declaration.path);
  }
  if (options.entrypoints.length > SCRIPT_LIMITS.modules) issue('module-limit');
  for (const entry of options.entrypoints.slice(0, SCRIPT_LIMITS.modules)) {
    if (!current()) return result();
    if (!/\.[cm]?tsx?$/.test(entry)) { issue('unsupported-entry', entry); continue; }
    const fileName = await resolve([paths.resolve(root, entry)]);
    if (!fileName) { issue('unresolved-entry', entry); continue; }
    entrypoints.push(fileName);
    schedule(fileName, 0);
  }
  for (let index = 0; index < queue.length && current(); index++) {
    const { fileName, depth } = queue[index];
    if (bytes >= SCRIPT_LIMITS.bytes) { issue('byte-limit', fileName); break; }
    try {
      const snapshot = await cancellable(ticket,
        () => host.read(fileName, Math.min(SCRIPT_LIMITS.fileBytes, SCRIPT_LIMITS.bytes - bytes)));
      if (!current()) break;
      if (snapshot.fileName !== fileName || typeof snapshot.text !== 'string'
        || !['string', 'number'].includes(typeof snapshot.version)) { issue('invalid-snapshot', fileName); continue; }
      // Recheck canonical identity after asynchronous reads, including editor-host reads.
      if (paths.resolve(await cancellable(ticket, () => host.realpath(fileName))) !== fileName) { issue('changed-path', fileName); continue; }
      if (!current()) break;
      const size = Buffer.byteLength(snapshot.text, 'utf8');
      if (size > SCRIPT_LIMITS.fileBytes) { issue('file-limit', fileName); continue; }
      if (bytes + size > SCRIPT_LIMITS.bytes) { issue('byte-limit', fileName); continue; }
      // Rejected syntax still consumed the input budget; otherwise hostile files could bypass it.
      bytes += size;
      const sourceFile = ts.createSourceFile(fileName, snapshot.text, ts.ScriptTarget.ES2022, true);
      const children = syntaxNodes(sourceFile);
      if (nodes + children.length > SCRIPT_LIMITS.nodes) { issue('syntax-limit', fileName); break; }
      nodes += children.length;
      modules.set(fileName, { ...snapshot, sourceFile, depth });
      if ((sourceFile as ts.SourceFile & { parseDiagnostics: readonly ts.Diagnostic[] }).parseDiagnostics.length)
        issue('syntax-error', fileName);
      if (sourceFile.referencedFiles.length || sourceFile.typeReferenceDirectives.length || sourceFile.libReferenceDirectives.length)
        issue('unsupported-reference-directive', fileName);
      for (const node of children) {
        if (!current()) break;
        if ((ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword)
          || (ts.isIdentifier(node) && ['require', 'eval', 'Function'].includes(node.text))
          || (ts.isElementAccessExpression(node) && ts.isStringLiteralLike(node.argumentExpression)
            && ['require', 'eval', 'Function'].includes(node.argumentExpression.text)))
          issue('dynamic-code', fileName, node.getStart(sourceFile));
        if (ts.isImportEqualsDeclaration(node) || ts.isImportTypeNode(node))
          issue('unsupported-import', fileName, node.getStart(sourceFile));
        if (!ts.isImportDeclaration(node) && !ts.isExportDeclaration(node)) continue;
        const literal = node.moduleSpecifier;
        if (!literal) continue;
        if (!ts.isStringLiteral(literal)) { issue('dynamic-import', fileName); continue; }
        const specifier = literal.text;
        if (SDK_MODULES.includes(specifier)) { edges.push({ from: fileName, specifier, external: true }); continue; }
        if (!/^\.\.?\//.test(specifier) || /[\\%\x00-\x1f:#?]/.test(specifier)) {
          issue('unsupported-import', fileName, literal.getStart(sourceFile)); continue;
        }
        const to = await resolve(candidates(paths, fileName, specifier));
        edges.push({ from: fileName, specifier, to });
        if (!to) { issue('unresolved-import', fileName, literal.getStart(sourceFile)); continue; }
        schedule(to, depth + 1);
        if (resolutions > SCRIPT_LIMITS.resolutions) break;
      }
    } catch (error) {
      issue(error instanceof Error && ['file-limit', 'syntax-limit', 'changed-during-read', 'stale-generation'].includes(error.message)
        ? error.message : 'unreadable-module', fileName);
    }
    // Let cancellation and editor changes run between modules; TypeScript itself stays synchronous.
    await new Promise<void>(resolve => setImmediate(resolve));
  }
  current();
  return result();
}

/** A compiler host backed only by the bounded snapshots; never ts.sys or tsconfig resolution. */
export function createSnapshotProgram(graph: ScriptGraph): ts.Program {
  const compilerOptions: ts.CompilerOptions = { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Node10, noLib: true, noResolve: true, types: [], skipLibCheck: true };
  const host: ts.CompilerHost = {
    getSourceFile: fileName => graph.modules.get(fileName)?.sourceFile,
    getDefaultLibFileName: () => '', writeFile: () => { throw new Error('Analysis cannot emit'); },
    getCurrentDirectory: () => graph.root, getDirectories: () => [],
    getCanonicalFileName: fileName => fileName, useCaseSensitiveFileNames: () => true, getNewLine: () => '\n',
    fileExists: fileName => graph.modules.has(fileName), readFile: fileName => graph.modules.get(fileName)?.text,
    resolveModuleNames: (names, containingFile) => names.map(name => {
      const to = moduleEdge(graph, containingFile, name)?.to;
      return to && graph.modules.has(to) ? { resolvedFileName: to } : undefined;
    }),
  };
  return ts.createProgram([...graph.modules.keys()], compilerOptions, host);
}
