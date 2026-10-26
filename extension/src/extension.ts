import * as vscode from 'vscode';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { spawn } from 'node:child_process';
import { LanguageClient, State, type ServerOptions } from 'vscode-languageclient/node';
import { registerSdkProjectCommands } from './sdkProjects';

interface ClassRecord {
  path: string;
  name?: string;
  kind?: string;
  super?: string;
  source?: { ref?: string };
  propertyCount?: number;
}
interface ClassCatalog {
  status?: string;
  reason?: string;
  classes?: ClassRecord[];
}

let client: LanguageClient | undefined;
let clientStart: Promise<void> | undefined;
let context: vscode.ExtensionContext;
const output = vscode.window.createOutputChannel('KDataForge');
const lintDiagnostics = vscode.languages.createDiagnosticCollection('kdf-cli');

function configuration() {
  return vscode.workspace.getConfiguration('kdf');
}

function executable(setting: 'languageServerPath' | 'cliPath', bundled: string): string {
  const configured = configuration().get<string>(setting, '').trim();
  const candidate = configured || context.asAbsolutePath(path.join('bin', bundled + (process.platform === 'win32' ? '.exe' : '')));
  if (!fs.existsSync(candidate)) throw new Error(`${path.basename(candidate)} is missing. Reinstall KDataForge Tools or configure kdf.${setting}.`);
  return candidate;
}

async function startLanguageServer(): Promise<LanguageClient | undefined> {
  if (!vscode.workspace.workspaceFolders?.length) return undefined;
  const command = executable('languageServerPath', 'kdf-language-server');
  const datapackPath = configuration().get<string>('datapackPath', '');
  const server: ServerOptions = () => Promise.resolve(spawn(command, [], {
    cwd: vscode.workspace.workspaceFolders![0].uri.fsPath,
    windowsHide: true,
    stdio: 'pipe',
    env: { ...process.env, KDF_EFFECTIVE_DATAPACK_PATH: datapackPath },
  }));
  const next = new LanguageClient('kdf', 'KDataForge YAML', server, {
    documentSelector: [{ language: 'yaml', scheme: 'file' }],
    initializationOptions: { kdf: { datapackPath, clientCommands: true } },
    synchronize: { configurationSection: 'kdf' },
    outputChannel: output,
  });
  next.onDidChangeState(({ newState }) => {
    if (newState === State.Stopped && client === next) output.appendLine('Language server stopped.');
  });
  client = next;
  clientStart = next.start();
  await clientStart;
  return next;
}

async function restartLanguageServer(): Promise<void> {
  const previous = client;
  client = undefined;
  clientStart = undefined;
  await previous?.stop();
  await startLanguageServer();
}

async function activeClient(): Promise<LanguageClient> {
  const running = client ?? (await startLanguageServer());
  if (!running) throw new Error('Open a filesystem workspace first.');
  await clientStart;
  return running;
}

async function selectClass(): Promise<ClassRecord | undefined> {
  const catalog = await (await activeClient()).sendRequest<ClassCatalog>('kdf/classes', {});
  const classes = Array.isArray(catalog.classes) ? catalog.classes.filter((item) => item?.path) : [];
  if (!classes.length) {
    void vscode.window.showWarningMessage(catalog.reason || 'No reflection datapack is bound.');
    return undefined;
  }
  const selected = await vscode.window.showQuickPick(
    classes.map((item) => ({
      label: item.name || item.path.split(/[./]/).at(-1) || item.path,
      description: item.kind || item.source?.ref || '',
      detail: item.path,
      item,
    })),
    { matchOnDescription: true, matchOnDetail: true, placeHolder: 'Search reflected classes' },
  );
  return selected?.item;
}

async function insertClass(): Promise<void> {
  const selected = await selectClass();
  if (!selected) return;
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    await vscode.env.clipboard.writeText(selected.path);
    void vscode.window.showInformationMessage('Class path copied to clipboard.');
    return;
  }
  await editor.edit((edit) => edit.insert(editor.selection.active, selected.path));
}

async function openCodex(): Promise<void> {
  const selected = await selectClass();
  if (!selected) return;
  const result = await (await activeClient()).sendRequest<{ class?: ClassRecord }>('kdf/classInfo', { path: selected.path });
  const item = result.class ?? selected;
  const content = [
    `# ${item.name || item.path}`,
    '',
    `\`${item.path}\``,
    '',
    item.kind ? `- Kind: ${item.kind}` : '',
    item.source?.ref ? `- Source: ${item.source.ref}` : '',
    item.super ? `- Parent: \`${item.super}\`` : '',
    typeof item.propertyCount === 'number' ? `- Properties: ${item.propertyCount}` : '',
  ].filter(Boolean).join('\n');
  const document = await vscode.workspace.openTextDocument({ language: 'markdown', content });
  await vscode.window.showTextDocument(document, { preview: true });
}

function severity(value: unknown): vscode.DiagnosticSeverity {
  return value === 'warning' ? vscode.DiagnosticSeverity.Warning
    : value === 'information' || value === 'info' ? vscode.DiagnosticSeverity.Information
      : vscode.DiagnosticSeverity.Error;
}

async function lint(target: vscode.Uri): Promise<void> {
  if (target.scheme !== 'file') throw new Error('Lint requires a filesystem target.');
  const command = executable('cliPath', 'kdf');
  const datapack = configuration().get<string>('datapackPath', '').trim();
  const args = ['lint', target.fsPath, '--format', 'json', '--no-color', ...(datapack ? ['--datapack', datapack] : [])];
  output.clear();
  output.appendLine(`$ ${path.basename(command)} ${args.join(' ')}`);
  const child = spawn(command, args, { windowsHide: true, stdio: 'pipe' });
  let stdout = '', stderr = '';
  child.stdout.on('data', (data) => { stdout += data.toString(); });
  child.stderr.on('data', (data) => { stderr += data.toString(); });
  const exitCode = await new Promise<number>((resolve, reject) => {
    child.on('error', reject);
    child.on('exit', (code) => resolve(code ?? 3));
  });
  output.append(stdout);
  output.append(stderr);
  lintDiagnostics.clear();
  try {
    const report = JSON.parse(stdout) as { diagnostics?: Array<Record<string, any>> };
    const grouped = new Map<string, vscode.Diagnostic[]>();
    for (const item of report.diagnostics ?? []) {
      const uri = item.location?.uri || (item.file ? vscode.Uri.file(item.file).toString() : undefined);
      if (!uri) continue;
      const range = item.range ?? item.location?.range ?? { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } };
      const diagnostic = new vscode.Diagnostic(
        new vscode.Range(range.start.line, range.start.character, range.end.line, range.end.character),
        String(item.message ?? 'KDataForge diagnostic'),
        severity(item.severity),
      );
      diagnostic.code = item.ruleId ?? item.code;
      diagnostic.source = 'kdf';
      const key = String(uri);
      grouped.set(key, [...(grouped.get(key) ?? []), diagnostic]);
    }
    for (const [uri, diagnostics] of grouped) lintDiagnostics.set(vscode.Uri.parse(uri), diagnostics);
  } catch (error) {
    output.appendLine(`Cannot parse lint JSON: ${String(error)}`);
  }
  await vscode.commands.executeCommand('workbench.actions.view.problems');
  if (exitCode > 1) output.show(true);
}

function command(action: () => Promise<void>): () => Promise<void> {
  return () => action().catch((error) => void vscode.window.showErrorMessage(String(error)));
}

export async function activate(extensionContext: vscode.ExtensionContext): Promise<void> {
  context = extensionContext;
  context.subscriptions.push(output, lintDiagnostics, ...registerSdkProjectCommands(context));
  context.subscriptions.push(
    vscode.commands.registerCommand('kdf.codex', command(openCodex)),
    vscode.commands.registerCommand('kdf.catalog', command(insertClass)),
    vscode.commands.registerCommand('kdf.lintFile', (uri?: vscode.Uri) => command(() => lint(uri ?? vscode.window.activeTextEditor?.document.uri!))()),
    vscode.commands.registerCommand('kdf.lintProject', command(async () => {
      const folder = vscode.workspace.workspaceFolders?.[0];
      if (!folder) throw new Error('Open a workspace first.');
      await lint(folder.uri);
    })),
    vscode.commands.registerCommand('kdf.lintOutput', () => output.show(true)),
    vscode.commands.registerCommand('kdf.switchDatapack', command(async () => {
      const selected = await vscode.window.showOpenDialog({ canSelectFiles: true, canSelectFolders: false, canSelectMany: false, filters: { 'KDataForge reflection': ['kdfdata'] } });
      if (!selected?.[0]) return;
      await configuration().update('datapackPath', selected[0].fsPath, vscode.ConfigurationTarget.Workspace);
      await restartLanguageServer();
    })),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('kdf.languageServerPath') || event.affectsConfiguration('kdf.datapackPath'))
        void restartLanguageServer().catch((error) => vscode.window.showErrorMessage(String(error)));
    }),
  );
  await startLanguageServer().catch((error) => void vscode.window.showErrorMessage(String(error)));
}

export async function deactivate(): Promise<void> {
  const previous = client;
  client = undefined;
  clientStart = undefined;
  await previous?.stop();
}
