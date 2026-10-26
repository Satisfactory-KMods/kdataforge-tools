import * as fs from 'node:fs';
import * as path from 'node:path';
import { randomBytes } from 'node:crypto';
import * as vscode from 'vscode';
import {
  newScriptContent,
  projectRef,
  renderSdkTemplate,
  scriptFilename,
} from './sdkProjectModel';
const SDK_MARKER = '.kdataforge-bundled-sdk.json';

interface BundledSdkManifest {
  version: string;
}

async function exists(file: string): Promise<boolean> {
  try {
    await fs.promises.access(file);
    return true;
  } catch {
    return false;
  }
}

async function selectedDirectory(uri?: vscode.Uri): Promise<string> {
  const selected = uri ?? vscode.window.activeTextEditor?.document.uri;
  if (selected) {
    if (selected.scheme !== 'file') throw new Error('KDataForgeJS actions require a filesystem target.');
    const stat = await fs.promises.stat(selected.fsPath);
    return stat.isDirectory() ? selected.fsPath : path.dirname(selected.fsPath);
  }
  const folders = vscode.workspace.workspaceFolders ?? [];
  if (folders.length === 1) return folders[0].uri.fsPath;
  const picked = await vscode.window.showWorkspaceFolderPick({ placeHolder: 'Select project folder' });
  if (!picked) throw new Error('Select a workspace folder first.');
  return picked.uri.fsPath;
}

async function copyTemplate(source: string, target: string, values: Readonly<Record<string, string>>): Promise<void> {
  await fs.promises.mkdir(target, { recursive: true });
  for (const entry of await fs.promises.readdir(source, { withFileTypes: true })) {
    const destinationName = entry.name === '_gitignore' ? '.gitignore' : entry.name;
    const from = path.join(source, entry.name);
    const to = path.join(target, destinationName);
    if (entry.isDirectory()) await copyTemplate(from, to, values);
    else if (entry.isFile()) {
      const content = await fs.promises.readFile(from, 'utf8');
      await fs.promises.writeFile(to, renderSdkTemplate(content, values), 'utf8');
    }
  }
}

function bundledSdk(context: vscode.ExtensionContext): string {
  return path.join(context.extensionPath, 'sdk', 'kdataforgejs');
}

async function sdkManifest(context: vscode.ExtensionContext): Promise<BundledSdkManifest> {
  const content = await fs.promises.readFile(path.join(bundledSdk(context), 'manifest.json'), 'utf8');
  const manifest = JSON.parse(content) as Partial<BundledSdkManifest>;
  if (typeof manifest.version !== 'string' || !manifest.version) throw new Error('Bundled KDataForgeJS SDK manifest is invalid.');
  return { version: manifest.version };
}

async function installSdk(context: vscode.ExtensionContext, projectRoot: string): Promise<string> {
  const source = path.join(bundledSdk(context), 'types');
  const target = path.join(projectRoot, 'node_modules', '@sf-kmods', 'kdataforgejs');
  const marker = path.join(target, SDK_MARKER);
  if (await exists(target) && !(await exists(marker)))
    throw new Error(`Refusing to replace unmanaged SDK directory: ${target}`);
  const temporary = `${target}.tmp-${randomBytes(6).toString('hex')}`;
  await fs.promises.rm(temporary, { recursive: true, force: true });
  await fs.promises.mkdir(path.dirname(target), { recursive: true });
  try {
    await fs.promises.cp(source, temporary, { recursive: true, errorOnExist: true });
    await fs.promises.writeFile(
      path.join(temporary, SDK_MARKER),
      JSON.stringify(await sdkManifest(context), null, 2) + '\n',
      'utf8',
    );
    await fs.promises.rm(target, { recursive: true, force: true });
    await fs.promises.rename(temporary, target);
  } catch (error) {
    await fs.promises.rm(temporary, { recursive: true, force: true });
    throw error;
  }
  return target;
}

async function findProjectRoot(start: string): Promise<string> {
  let current = path.resolve(start);
  for (;;) {
    if ((await exists(path.join(current, 'package.json'))) && (await exists(path.join(current, 'DataForge'))))
      return current;
    const parent = path.dirname(current);
    if (parent === current) throw new Error('No KDataForgeJS project found.');
    current = parent;
  }
}

async function findPackRoot(start: string): Promise<string> {
  let current = path.resolve(start);
  for (;;) {
    if (await exists(path.join(current, 'pack.yml'))) return current;
    if (await exists(path.join(current, 'DataForge', 'pack.yml'))) return path.join(current, 'DataForge');
    const parent = path.dirname(current);
    if (parent === current) throw new Error('No DataForge/pack.yml found.');
    current = parent;
  }
}

async function openText(file: string): Promise<void> {
  const document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  await vscode.window.showTextDocument(document, { preview: false });
}

async function createProject(context: vscode.ExtensionContext, uri?: vscode.Uri): Promise<void> {
  const parent = await selectedDirectory(uri);
  const name = await vscode.window.showInputBox({
    title: 'Create KDataForgeJS Project',
    prompt: 'Project directory and pack ref',
    value: 'kdataforge-project',
    validateInput: (value) => {
      try { projectRef(value); return undefined; }
      catch (error) { return String(error); }
    },
  });
  if (name === undefined) return;
  const ref = projectRef(name);
  const target = path.join(parent, name);
  if (path.dirname(target) !== path.resolve(parent) || (await exists(target)))
    throw new Error(`Project target already exists or is invalid: ${target}`);
  const manifest = await sdkManifest(context);
  const temporary = path.join(parent, `.${path.basename(target)}.kdf-tmp-${randomBytes(6).toString('hex')}`);
  try {
    await copyTemplate(path.join(bundledSdk(context), 'template'), temporary, {
      ref,
      sdkVersion: manifest.version,
    });
    await installSdk(context, temporary);
    await fs.promises.rename(temporary, target);
  } catch (error) {
    await fs.promises.rm(temporary, { recursive: true, force: true });
    throw error;
  }
  await openText(path.join(target, 'DataForge', 'scripts', 'main.ts'));
  void vscode.window.showInformationMessage(`Created KDataForgeJS project ${ref}.`);
}

async function createScript(uri?: vscode.Uri): Promise<void> {
  const packRoot = await findPackRoot(await selectedDirectory(uri));
  const name = await vscode.window.showInputBox({
    title: 'Create KDataForgeJS Script',
    prompt: 'Script file name',
    value: 'handler.ts',
    validateInput: (value) => {
      try { scriptFilename(value); return undefined; }
      catch (error) { return String(error); }
    },
  });
  if (name === undefined) return;
  const target = path.join(packRoot, 'scripts', scriptFilename(name));
  if (await exists(target)) throw new Error(`Script already exists: ${target}`);
  await fs.promises.mkdir(path.dirname(target), { recursive: true });
  await fs.promises.writeFile(target, newScriptContent(), { encoding: 'utf8', flag: 'wx' });
  await openText(target);
}

async function refreshSdk(context: vscode.ExtensionContext, uri?: vscode.Uri): Promise<void> {
  const projectRoot = await findProjectRoot(await selectedDirectory(uri));
  await installSdk(context, projectRoot);
  void vscode.window.showInformationMessage('Built-in KDataForgeJS SDK refreshed.');
}

export function registerSdkProjectCommands(context: vscode.ExtensionContext): vscode.Disposable[] {
  const run = (action: () => Promise<void>) => action().catch((error) => vscode.window.showErrorMessage(String(error)));
  return [
    vscode.commands.registerCommand('kdf.sdk.createProject', (uri?: vscode.Uri) => run(() => createProject(context, uri))),
    vscode.commands.registerCommand('kdf.sdk.createScript', (uri?: vscode.Uri) => run(() => createScript(uri))),
    vscode.commands.registerCommand('kdf.sdk.refresh', (uri?: vscode.Uri) => run(() => refreshSdk(context, uri))),
  ];
}
