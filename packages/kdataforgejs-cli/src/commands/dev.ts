import { watch, readFileSync, readdirSync, existsSync, type FSWatcher } from "node:fs";
import { join, resolve, sep } from "node:path";
import { discoverPacks } from "../pack/discover.js";
import { checkProject } from "./check.js";
import { lintProject } from "./lint.js";
import { syncProject } from "./sync.js";
import { validateProject } from "./validate.js";
import { formatDiagnostic, hasErrors } from "../diagnostics.js";
import type { Io } from "../cli.js";

const IGNORED = new Set(["node_modules", "typescript", ".kdataforge", ".git"]);
const DEBOUNCE_MS = 200;

/** Watches authored files. Generated manifests and declarations cannot feed the watcher. */
export function startDev(directory: string, io: Io, packRef?: string): { close: () => void } {
	const selection = discoverPacks(directory, packRef);
	const projects = [...new Set(selection.packs.map((pack) => pack.projectRoot))];
	const watchers = projects.map((project) => startProjectDev(project, projects.length === 1 ? io : {
		out: (line) => io.out(`${project}: ${line}`), err: (line) => io.err(`${project}: ${line}`),
	}, packRef));
	return { close: () => { for (const watcher of watchers) watcher.close(); } };
}

function startProjectDev(directory: string, io: Io, packRef?: string): { close: () => void } {
	const root = resolve(directory);
	const dataForge = join(root, "DataForge");
	if (!existsSync(dataForge)) throw new Error(`DataForge directory not found in ${root}`);
	const watchers = new Map<string, FSWatcher>();
	let timer: NodeJS.Timeout | undefined;
	let running = false;
	let dirty = false;
	let closed = false;
	const generatedManifests = new Map<string, string>();
	const schedule = (): void => {
		if (timer) clearTimeout(timer);
		timer = setTimeout(() => { dirty = true; void run(); }, DEBOUNCE_MS);
	};

	const attach = (directoryPath: string): void => {
		if (!existsSync(directoryPath)) return;
		if (!watchers.has(directoryPath)) watchers.set(directoryPath, watch(directoryPath, (_event, filename) => {
			if (closed) return;
			const name = filename?.toString();
			if (name !== undefined && IGNORED.has(name)) return;
			if (name === "pack.yml") {
				const path = join(directoryPath, name);
				if (existsSync(path) && generatedManifests.get(path) === readFileSync(path, "utf8")) return;
			}
			refresh();
			schedule();
		}));
		for (const entry of readdirSync(directoryPath, { withFileTypes: true })) {
			if (entry.isDirectory() && !IGNORED.has(entry.name)) attach(join(directoryPath, entry.name));
		}
	};
	const refresh = (): void => {
		for (const path of [...watchers.keys()]) {
			if (!existsSync(path)) { watchers.get(path)?.close(); watchers.delete(path); }
		}
		attach(dataForge);
	};
	const run = async (): Promise<void> => {
		if (running || closed) return;
		running = true;
		while (dirty && !closed) {
			dirty = false;
			try {
				syncProject(root, packRef);
				for (const pack of discoverPacks(root, packRef).packs.map((item) => item.root)) {
					const path = join(pack, "pack.yml");
					generatedManifests.set(path, readFileSync(path, "utf8"));
				}
				const diagnostics = [...checkProject(root, undefined, packRef), ...lintProject(root, packRef), ...validateProject(root, packRef)];
				for (const diagnostic of diagnostics) io.err(formatDiagnostic(diagnostic));
				if (hasErrors(diagnostics)) io.err("Checks failed; reload skipped.");
				else io.out("Checks passed. Run KDF.Reload in local game or server console.");
			} catch (error) {
				io.err(error instanceof Error ? error.message : String(error));
			}
		}
		running = false;
	};
	attach(dataForge);
	watchers.set(root, watch(root, (_event, filename) => {
		if (closed || !filename) return;
		if (filename.toString() === "DataForge") {
			for (const path of [...watchers.keys()]) {
				if (path === dataForge || path.startsWith(`${dataForge}${sep}`)) {
					watchers.get(path)?.close();
					watchers.delete(path);
				}
			}
			refresh();
			schedule();
		} else if (/^(?:tsconfig(?:\.kdf)?\.json|package\.json)$/u.test(filename.toString())) schedule();
	}));
	io.out(`Watching ${dataForge}`);
	dirty = true;
	void run();
	return { close: () => {
		closed = true;
		if (timer) clearTimeout(timer);
		for (const watcher of watchers.values()) watcher.close();
		watchers.clear();
	} };
}
