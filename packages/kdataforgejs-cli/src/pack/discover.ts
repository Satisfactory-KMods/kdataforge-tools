import { existsSync, lstatSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { parse } from "yaml";
import { readManifest } from "./manifest.js";

const SKIPPED = new Set(["node_modules", ".git", ".kdataforge", "dist", "dist-test"]);

export interface DiscoveredPack {
	readonly root: string;
	readonly projectRoot: string;
	readonly ref: string;
}

export interface PackSelection {
	readonly root: string;
	readonly packs: readonly DiscoveredPack[];
	readonly workspace: boolean;
}

/** Resolve authored packs without running package-manager or workspace code. */
export function discoverPacks(directory: string, packRef?: string): PackSelection {
	const root = resolve(directory);
	if (!existsSync(root) || !lstatSync(root).isDirectory()) throw new Error(`Project directory does not exist: ${root}`);
	const workspaceRoots = workspaceProjects(root);
	const projects = workspaceRoots === undefined ? [root] : [root, ...workspaceRoots];
	const packs: DiscoveredPack[] = [];
	const seenPaths = new Set<string>();
	for (const projectRoot of projects) {
		const dataForge = join(projectRoot, "DataForge");
		const start = existsSync(dataForge) ? dataForge : workspaceRoots === undefined ? projectRoot : undefined;
		if (start === undefined) continue;
		for (const packRoot of findRoots(start)) {
			const actual = realpathSync(packRoot);
			if (!isInside(projectRoot, actual) || !isInside(root, actual)) throw new Error(`Pack escapes project: ${packRoot}`);
			if (seenPaths.has(actual)) continue;
			seenPaths.add(actual);
			const manifestPath = join(packRoot, "pack.yml");
			if (lstatSync(manifestPath).isSymbolicLink() || !isInside(actual, realpathSync(manifestPath))) {
				throw new Error(`Pack manifest must be a regular file inside its pack: ${manifestPath}`);
			}
			const ref = readManifest(packRoot).ref;
			if (!ref) throw new Error(`Pack ref is missing: ${manifestPath}`);
			packs.push({ root: packRoot, projectRoot, ref });
		}
	}
	packs.sort((a, b) => a.root.localeCompare(b.root));
	const refs = new Map<string, string>();
	for (const pack of packs) {
		const key = pack.ref.toLowerCase();
		const previous = refs.get(key);
		if (previous) throw new Error(`Duplicate pack ref '${pack.ref}': ${previous} and ${pack.root}`);
		refs.set(key, pack.root);
	}
	if (packs.length === 0) throw new Error(`No pack.yml found under ${root}.`);
	const selected = packRef === undefined ? packs : packs.filter((pack) => pack.ref === packRef);
	if (selected.length === 0) throw new Error(`No pack with ref '${packRef}' found under ${root}.`);
	return { root, packs: selected, workspace: workspaceRoots !== undefined };
}

/** Retained for callers that need only pack roots. */
export function findPackRoots(projectRoot: string): string[] {
	return discoverPacks(projectRoot).packs.map((pack) => pack.root);
}

/** Child-project commands share their containing workspace's writer lock. */
export function workspaceRootFor(directory: string): string {
	const root = resolve(directory);
	let ancestor = dirname(root);
	while (ancestor !== dirname(ancestor)) {
		const projects = workspaceProjects(ancestor);
		if (projects?.some((project) => isInside(project, root))) return ancestor;
		ancestor = dirname(ancestor);
	}
	return root;
}

export function isInside(root: string, target: string): boolean {
	const path = relative(root, target);
	return path === "" || (path !== ".." && !path.startsWith(`..${sep}`) && !isAbsolute(path));
}

function findRoots(start: string): string[] {
	const roots: string[] = [];
	const walk = (directory: string): void => {
		if (lstatSync(directory).isSymbolicLink()) return;
		if (existsSync(join(directory, "pack.yml"))) roots.push(directory);
		for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
			if (entry.isDirectory() && !SKIPPED.has(entry.name) && !entry.name.startsWith(".")) walk(join(directory, entry.name));
		}
	};
	walk(start);
	return roots;
}

function workspaceProjects(root: string): string[] | undefined {
	const packageFile = join(root, "package.json");
	const packageJson = existsSync(packageFile) ? JSON.parse(readFileSync(packageFile, "utf8")) as Record<string, unknown> : {};
	const npmValue = packageJson.workspaces;
	const npmPatterns = Array.isArray(npmValue) ? npmValue :
		isRecord(npmValue) && Array.isArray(npmValue.packages) ? npmValue.packages : undefined;
	const pnpmFile = join(root, "pnpm-workspace.yaml");
	const pnpmValue = existsSync(pnpmFile) ? parse(readFileSync(pnpmFile, "utf8")) as unknown : undefined;
	const pnpmPatterns = isRecord(pnpmValue) ? pnpmValue.packages : undefined;
	if (npmPatterns === undefined && pnpmPatterns === undefined) return undefined;
	const npmRoots = npmPatterns === undefined ? undefined : expandPatterns(root, npmPatterns);
	const pnpmRoots = pnpmPatterns === undefined ? undefined : expandPatterns(root, pnpmPatterns);
	if (npmRoots && pnpmRoots && (npmRoots.length !== pnpmRoots.length || npmRoots.some((path, i) => path !== pnpmRoots[i]))) {
		throw new Error("package.json workspaces and pnpm-workspace.yaml select different directories.");
	}
	return npmRoots ?? pnpmRoots ?? [];
}

function expandPatterns(root: string, value: unknown): string[] {
	if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) throw new Error("Workspace packages must be string paths.");
	const projects = new Set<string>();
	for (const pattern of value as string[]) {
		const segments = pattern.split("/");
		if (isAbsolute(pattern) || segments.some((part) => part === "" || part === "." || part === ".." ||
			(part.includes("*") && part !== "*") || /[?{}[\]\\]/u.test(part)) || segments.filter((part) => part === "*").length > 1) {
			throw new Error(`Unsupported workspace pattern '${pattern}'. Use a literal path or one '*' segment.`);
		}
		const star = segments.indexOf("*");
		const candidates = star < 0 ? [join(root, ...segments)] :
			!existsSync(join(root, ...segments.slice(0, star))) ? [] :
			readdirSync(join(root, ...segments.slice(0, star)), { withFileTypes: true })
				.filter((entry) => (entry.isDirectory() || entry.isSymbolicLink()) && !entry.name.startsWith("."))
				.map((entry) => join(root, ...segments.slice(0, star), entry.name, ...segments.slice(star + 1)));
		for (const candidate of candidates) {
			if (!existsSync(candidate)) continue;
			if (lstatSync(candidate).isSymbolicLink() || !lstatSync(candidate).isDirectory() || !isInside(root, realpathSync(candidate))) {
				throw new Error(`Workspace package escapes root: ${candidate}`);
			}
			projects.add(candidate);
		}
	}
	return [...projects].sort();
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}
