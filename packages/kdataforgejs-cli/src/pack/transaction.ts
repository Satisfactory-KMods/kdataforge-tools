import { closeSync, existsSync, lstatSync, mkdirSync, openSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";
import { workspaceRootFor } from "./discover.js";

export interface StagedPath {
	readonly target: string;
	readonly stage: string;
}

/** Serializes CLI writers. A stale lock needs deliberate removal after checking no writer remains. */
export function withProjectLock<T>(root: string, run: () => T): T {
	if (!existsSync(root) || !statSync(root).isDirectory()) throw new Error(`Project directory does not exist: ${root}`);
	const lockRoot = workspaceRootFor(root);
	const directory = join(lockRoot, ".kdataforge");
	assertSafeWrite(lockRoot, join(directory, "writer.lock"));
	mkdirSync(directory, { recursive: true });
	const lock = join(directory, "writer.lock");
	let handle: number;
	try { handle = openSync(lock, "wx"); }
	catch { throw new Error(`KDF writer lock exists: ${lock}. Check for a running writer before removing it.`); }
	try { return run(); }
	finally { closeSync(handle); rmSync(lock, { force: true }); }
}

export async function withProjectLockAsync<T>(root: string, run: () => Promise<T>): Promise<T> {
	if (!existsSync(root) || !statSync(root).isDirectory()) throw new Error(`Project directory does not exist: ${root}`);
	const lockRoot = workspaceRootFor(root);
	const directory = join(lockRoot, ".kdataforge");
	assertSafeWrite(lockRoot, join(directory, "writer.lock"));
	mkdirSync(directory, { recursive: true });
	const lock = join(directory, "writer.lock");
	let handle: number;
	try { handle = openSync(lock, "wx"); }
	catch { throw new Error(`KDF writer lock exists: ${lock}. Check for a running writer before removing it.`); }
	try { return await run(); }
	finally { closeSync(handle); rmSync(lock, { force: true }); }
}

export function stageText(target: string, content: string): StagedPath {
	mkdirSync(dirname(target), { recursive: true });
	const stage = `${target}.${randomUUID()}.stage`;
	writeFileSync(stage, content, { flag: "wx" });
	return { target, stage };
}

export function stageTexts(files: readonly { readonly path: string; readonly content: string }[]): StagedPath[] {
	const stages: StagedPath[] = [];
	try {
		for (const file of files) stages.push(stageText(file.path, file.content));
		return stages;
	} catch (error) {
		for (const stage of stages) rmSync(stage.stage, { force: true });
		throw error;
	}
}

export function assertSafeWrite(root: string, target: string): void {
	const base = resolve(root);
	let path = resolve(target);
	const relativePath = relative(base, path);
	if (relativePath === ".." || relativePath.startsWith(`..${sep}`)) throw new Error(`Write escapes project: ${target}`);
	while (path !== base) {
		if (existsSync(path) && lstatSync(path).isSymbolicLink()) throw new Error(`Write path contains symlink: ${path}`);
		path = dirname(path);
	}
}

/** Staged files/directories swap under the writer lock; ordinary failures restore old paths. */
export function commitStaged(paths: readonly StagedPath[]): void {
	const committed: Array<{ target: string; backup?: string }> = [];
	try {
		for (const { target, stage } of paths) {
			const backup = existsSync(target) ? `${target}.${randomUUID()}.backup` : undefined;
			if (backup) renameSync(target, backup);
			committed.push({ target, ...(backup ? { backup } : {}) });
			renameSync(stage, target);
		}
	} catch (error) {
		for (const item of committed.reverse()) {
			rmSync(item.target, { recursive: true, force: true });
			if (item.backup) renameSync(item.backup, item.target);
		}
		throw error;
	} finally {
		for (const item of paths) rmSync(item.stage, { recursive: true, force: true });
	}
	for (const item of committed) if (item.backup) rmSync(item.backup, { recursive: true, force: true });
}
