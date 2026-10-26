import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  AnalysisGeneration,
  analyzeScripts,
  buildCapabilityLensModel,
  buildCompletionCandidates,
  buildHoverModel,
  buildPackFileHoverModel,
  buildReplacementQuickFix,
  planTypeScriptSymbolRename,
  readApiPolicyMetadata,
  type ApiPolicyMetadata,
  type IdentifierCatalog,
  type ScriptAnalysis,
} from './scriptAnalysis';
import { buildFormModel, planFormEdit } from './contentForms/formModel';
import { contentFormHtml } from './contentForms/webview';
import { exportDiagnosticsJson, exportDiagnosticsSarif, type StructuredDiagnostic } from './diagnosticExport';
import { previewText } from './contentPreview';

type Request = (method: string, params: unknown) => Promise<unknown>;
const selector: vscode.DocumentSelector = [{ language: 'typescript', scheme: 'file' }, { language: 'javascript', scheme: 'file' }];

function offset(document: vscode.TextDocument, position: vscode.Position): number {
  return document.offsetAt(position);
}

function range(document: vscode.TextDocument, value: { start: number; end: number }): vscode.Range {
  return new vscode.Range(document.positionAt(value.start), document.positionAt(value.end));
}

function dataForgeRoot(fileName: string): string | undefined {
  let current = path.dirname(fileName);
  while (path.dirname(current) !== current) {
    if (path.basename(current).toLowerCase() === 'dataforge') return current;
    current = path.dirname(current);
  }
  return;
}

function policy(extensionPath: string): ApiPolicyMetadata | undefined {
  const file = path.join(extensionPath, 'sdk', 'kdataforgejs', 'authoring-metadata.json');
  try { return readApiPolicyMetadata(fs.readFileSync(file, 'utf8')); } catch { return; }
}

function markdown(model: ReturnType<typeof buildHoverModel>): vscode.MarkdownString | undefined {
  if (!model) return;
  const known = <T>(fact: { status: string; value?: T; reason?: string }): string =>
    fact.status === 'known' ? String(fact.value) : `unknown (${fact.reason ?? 'not supplied'})`;
  const value = new vscode.MarkdownString(undefined, true);
  value.isTrusted = false;
  value.appendMarkdown(`**${model.label}**  \n`);
  value.appendMarkdown(`Policy: ${model.status}  \n`);
  value.appendMarkdown(`Capabilities: ${known(model.capabilities)}  \n`);
  value.appendMarkdown(`Authority: ${known(model.authority)}  \n`);
  value.appendMarkdown(`Lifecycle: ${known(model.lifecycle)}  \n`);
  value.appendMarkdown(`Quotas: ${known(model.quotas)}  \n`);
  value.appendMarkdown(`SDK: ${model.sdkVersion}; catalog: ${model.catalogVersion ?? 'unknown'}`);
  return value;
}

class AnalysisCache {
  private readonly generations = new Map<string, AnalysisGeneration>();
  private readonly cache = new Map<string, { version: number; analysis: Promise<ScriptAnalysis> }>();
  constructor(private readonly metadata: ApiPolicyMetadata) {}
  async get(document: vscode.TextDocument, token?: vscode.CancellationToken): Promise<ScriptAnalysis | undefined> {
    const root = dataForgeRoot(document.fileName);
    if (!root || !document.fileName.includes(`${path.sep}scripts${path.sep}`)) return;
    const cached = this.cache.get(document.uri.toString());
    if (cached?.version === document.version) return cached.analysis;
    const generation = this.generations.get(root) ?? new AnalysisGeneration();
    this.generations.set(root, generation);
    const controller = new AbortController();
    token?.onCancellationRequested(() => controller.abort());
    const files = await vscode.workspace.findFiles(new vscode.RelativePattern(path.join(root, 'scripts'), '**/*.{ts,tsx,js,mjs,cjs}'), '**/node_modules/**', 65);
    const entrypoints = [...new Set([document.fileName, ...files.map(file => file.fsPath)])].slice(0, 65);
    const analysis = analyzeScripts({ packRoot: root, entrypoints, allPackScriptsProvided: files.length < 65,
      ticket: generation.next(controller.signal) }, this.metadata);
    this.cache.set(document.uri.toString(), { version: document.version, analysis });
    return analysis;
  }
}

function diagnostic(item: vscode.Diagnostic, uri: vscode.Uri): StructuredDiagnostic {
  const code = typeof item.code === 'object' ? item.code.value : item.code;
  return { severity: item.severity, code: String(code ?? 'unknown'), message: item.message, file: uri.fsPath,
    source: item.source, range: { start: { line: item.range.start.line, character: item.range.start.character },
      end: { line: item.range.end.line, character: item.range.end.character } } };
}

async function writeExport(kind: 'json' | 'sarif'): Promise<void> {
  const rows = vscode.languages.getDiagnostics().flatMap(([uri, items]) => items.map(item => diagnostic(item, uri)));
  const uri = await vscode.window.showSaveDialog({ filters: kind === 'json' ? { JSON: ['json'] } : { SARIF: ['sarif', 'json'] },
    defaultUri: vscode.Uri.file(path.join(vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? '', `kdf-diagnostics.${kind}`)) });
  if (!uri) return;
  await vscode.workspace.fs.writeFile(uri, Buffer.from(kind === 'json' ? exportDiagnosticsJson(rows) : exportDiagnosticsSarif(rows)));
}

export function registerAuthoringFeatures(context: vscode.ExtensionContext, request: Request): vscode.Disposable[] {
  const metadata = policy(context.extensionPath);
  const disposables: vscode.Disposable[] = [];
  if (metadata) {
    const analyses = new AnalysisCache(metadata);
    disposables.push(
      vscode.languages.registerHoverProvider(selector, { async provideHover(document, position, token) {
        const analysis = await analyses.get(document, token); if (!analysis) return;
        const at = offset(document, position), model = buildHoverModel(analysis, document.fileName, at);
        const content = markdown(model);
        const file = buildPackFileHoverModel(analysis, document.fileName, at);
        if (!content && file) return new vscode.Hover(`Pack file: ${file.path ?? file.status} (${file.status})`, range(document, file.range));
        return content && model ? new vscode.Hover(content, range(document, model.range)) : undefined;
      } }),
      vscode.languages.registerCodeLensProvider(selector, { async provideCodeLenses(document, token) {
        const analysis = await analyses.get(document, token); if (!analysis) return [];
        const model = buildCapabilityLensModel(analysis, document.fileName); if (!model) return [];
        const title = model.complete ? `KDF capabilities: ${model.used.length} used, ${model.missing?.length ?? 0} missing`
          : `KDF capabilities: incomplete (${model.unknownSymbols.length} unknown)`;
        return [new vscode.CodeLens(new vscode.Range(0, 0, 0, 0), { title, command: 'kdf.authoring.capabilities', arguments: [model] })];
      } }),
      vscode.languages.registerCompletionItemProvider(selector, { async provideCompletionItems(document, position, token) {
        const analysis = await analyses.get(document, token); if (!analysis) return;
        const response = await request('kdf/contentCompletion', { kind: 'content', query: '', limit: 200 }).catch(() => undefined) as
          { items?: Array<{ label: string; insertText: string; kind: string }> } | undefined;
        const catalog: IdentifierCatalog = { sdkVersion: metadata.sdkVersion, catalogVersion: metadata.catalogVersion,
          identifiers: (response?.items ?? []).map(item => ({ id: item.insertText, label: item.label,
            kind: item.kind === 'class' ? 'class' : item.kind === 'asset' ? 'asset' : 'content' })) };
        const result = buildCompletionCandidates(analysis, document.fileName, offset(document, position), catalog);
        return new vscode.CompletionList(result.items.map(item => { const value = new vscode.CompletionItem(item.label);
          value.insertText = item.insertText; value.range = range(document, item.range); return value; }), result.isIncomplete);
      } }, '"', "'") ,
      vscode.languages.registerRenameProvider(selector, { async prepareRename(document, position, token) {
        const analysis = await analyses.get(document, token); if (!analysis) return;
        const plan = planTypeScriptSymbolRename(analysis, document.fileName, offset(document, position), '__kdf_probe__');
        return plan.blockers.includes('unknown-symbol-location') || plan.blockers.includes('symbol-rename-unproven') ? undefined : document.getWordRangeAtPosition(position);
      }, async provideRenameEdits(document, position, newName, token) {
        const analysis = await analyses.get(document, token); if (!analysis) return;
        const plan = planTypeScriptSymbolRename(analysis, document.fileName, offset(document, position), newName);
        if (!plan.safe) throw new Error(`Unsafe rename: ${plan.blockers.join(', ')}`);
        const edit = new vscode.WorkspaceEdit();
        for (const item of plan.edits) edit.replace(vscode.Uri.file(item.fileName), range(await vscode.workspace.openTextDocument(item.fileName), item), item.newText);
        return edit;
      } }),
      vscode.languages.registerCodeActionsProvider(selector, { async provideCodeActions(document, selected, _context, token) {
        const analysis = await analyses.get(document, token); if (!analysis) return [];
        const fix = buildReplacementQuickFix(analysis, document.fileName, offset(document, selected.start), metadata);
        if (!fix?.plan.safe) return [];
        const action = new vscode.CodeAction(fix.title, vscode.CodeActionKind.QuickFix); const edit = new vscode.WorkspaceEdit();
        for (const item of fix.plan.edits) edit.replace(vscode.Uri.file(item.fileName), range(await vscode.workspace.openTextDocument(item.fileName), item), item.newText);
        action.edit = edit; return [action];
      } }),
    );
  }
  disposables.push(
    vscode.commands.registerCommand('kdf.authoring.capabilities', model => vscode.window.showInformationMessage(JSON.stringify(model))),
    vscode.commands.registerCommand('kdf.diagnostics.exportJson', () => writeExport('json')),
    vscode.commands.registerCommand('kdf.diagnostics.exportSarif', () => writeExport('sarif')),
    vscode.commands.registerCommand('kdf.why', async () => {
      const editor = vscode.window.activeTextEditor; if (!editor) return;
      const result = await request('kdf/why', { textDocument: { uri: editor.document.uri.toString(), version: editor.document.version },
        position: { line: editor.selection.active.line, character: editor.selection.active.character } });
      const document = await vscode.workspace.openTextDocument({ language: 'json', content: JSON.stringify(result, null, 2) });
      await vscode.window.showTextDocument(document, { preview: true });
    }),
    vscode.commands.registerCommand('kdf.previewValue', async () => {
      const editor = vscode.window.activeTextEditor; if (!editor) return;
      const selected = editor.document.getText(editor.selection); let value: unknown = selected;
      try { value = JSON.parse(selected); } catch {}
      const preview = previewText(value, []);
      await vscode.window.showInformationMessage(preview.text || (preview.state === 'unresolved' ? preview.reason : 'No preview'));
    }),
    vscode.window.registerCustomEditorProvider('kdf.contentForm', { async resolveCustomTextEditor(document, panel) {
      const kind = path.basename(document.fileName).split('.').at(-2) ?? 'pack';
      const response = await request('kdf/effectiveSchema', { kind, textDocument: { uri: document.uri.toString(), version: document.version } }) as
        { schema?: unknown; resources?: Record<string, unknown> };
      const schemas = { ...(response.resources ?? {}), [kind]: response.schema } as Record<string, never>;
      const model = buildFormModel(document.getText(), document.uri.toString(), document.version, 0, schemas);
      const nonce = Buffer.from(crypto.getRandomValues(new Uint8Array(24))).toString('base64url');
      panel.webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(context.extensionUri, 'media')] };
      panel.webview.html = contentFormHtml(panel.webview.cspSource, nonce,
        panel.webview.asWebviewUri(vscode.Uri.joinPath(context.extensionUri, 'media', 'content-form.css')).toString(),
        panel.webview.asWebviewUri(vscode.Uri.joinPath(context.extensionUri, 'media', 'content-form.js')).toString());
      panel.webview.postMessage({ type: 'model', value: model });
      panel.webview.onDidReceiveMessage(async message => { const current = await vscode.workspace.openTextDocument(document.uri);
        const planned = planFormEdit(current.getText(), current.uri.toString(), current.version, schemas, message);
        const edit = new vscode.WorkspaceEdit(); edit.replace(current.uri,
          new vscode.Range(current.positionAt(planned.start), current.positionAt(planned.end)), planned.text);
        if (!await vscode.workspace.applyEdit(edit)) throw new Error('Content form edit was rejected'); });
    } }, { supportsMultipleEditorsPerDocument: false, webviewOptions: { retainContextWhenHidden: false } }),
  );
  context.subscriptions.push(...disposables);
  return disposables;
}
