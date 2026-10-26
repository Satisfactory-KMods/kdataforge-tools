import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, relative, resolve } from "node:path";
import { discoverPacks } from "../pack/discover.js";
import { assertSafeWrite } from "../pack/transaction.js";
import { inspectGameData } from "./gameData.js";

const PACKAGES = ["@sf-kmods/kdataforgejs", "@sf-kmods/kdataforgejs-cli"] as const;

export interface UpdateOptions {
	readonly directory: string;
	readonly apply?: boolean;
	readonly offline?: boolean;
	readonly gamePath?: string;
	readonly timeout?: string;
	readonly contentTypes?: boolean;
	readonly force?: boolean;
}

export interface UpdateDependencies {
	readonly run?: (command: string, args: readonly string[], cwd: string) => number;
	readonly installedVersion?: (project: string, name: string) => string | undefined;
	readonly localBin?: (project: string) => string;
}

export interface UpdateResult {
	readonly lines: readonly string[];
	readonly applied: boolean;
}

/** Check mode is read-only and network-free. Apply installs only fixed local dev dependencies. */
export function updateProject(options: UpdateOptions, dependencies: UpdateDependencies = {}): UpdateResult {
	const selection = discoverPacks(options.directory);
	const root = selection.root;
	const projects = [...new Set(selection.packs.map((pack) => pack.projectRoot))];
	const manager = packageManager(root);
	for (const project of projects) assertSafeWrite(root, join(project, "package.json"));
	assertSafeWrite(root, join(root, "package.json"));
	assertSafeWrite(root, join(root, "package-lock.json"));
	assertSafeWrite(root, join(root, "pnpm-lock.yaml"));
	if (selection.workspace) {
		const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as { workspaces?: unknown };
		if (manager === "npm" && manifest.workspaces === undefined ||
			manager === "pnpm" && !existsSync(join(root, "pnpm-workspace.yaml"))) {
			throw new Error(`${manager} workspace metadata is missing at ${root}; no packages were updated.`);
		}
	}
	const installedVersion = dependencies.installedVersion ?? localVersion;
	const lines: string[] = [];
	for (const project of projects) {
		const manifest = JSON.parse(readFileSync(join(project, "package.json"), "utf8")) as { devDependencies?: Record<string, string> };
		for (const name of PACKAGES) {
			lines.push(`${relative(root, project) || "."}: ${name} declared ${manifest.devDependencies?.[name] ?? "missing"}, installed ${installedVersion(project, name) ?? "missing"}`);
		}
	}
	const health = inspectGameData(root, projects, options.gamePath);
	lines.push(`Game data ${health.status}: ${health.message}`);
	lines.push("Latest registry versions are not queried in inspect mode.");
	const command = manager === "npm"
		? "npm install --ignore-scripts --save-dev @sf-kmods/kdataforgejs@latest @sf-kmods/kdataforgejs-cli@latest"
		: "pnpm add --ignore-scripts --save-dev @sf-kmods/kdataforgejs@latest @sf-kmods/kdataforgejs-cli@latest";
	if (!options.apply) {
		lines.push(`Run kdf update --apply to install locally and refresh game data; package command per pack: ${command}`);
		return { lines, applied: false };
	}
	const run = dependencies.run ?? runProcess;
	if (!options.offline) {
		for (const project of projects) {
			const args = manager === "npm"
				? ["install", "--ignore-scripts", "--save-dev", ...(selection.workspace ? ["--workspace", relative(root, project)] : []),
					...PACKAGES.map((name) => `${name}@latest`)]
				: ["--dir", project, "add", "--ignore-scripts", "--save-dev", ...PACKAGES.map((name) => `${name}@latest`)];
			if (run(manager, args, manager === "npm" ? root : project) !== 0) {
				throw new Error(`Package-manager update failed in ${project}. Inspect its manifest and root lockfile before retrying; game data was not refreshed.`);
			}
		}
	} else if (projects.some((project) => PACKAGES.some((name) => installedVersion(project, name) === undefined))) {
		throw new Error("Offline refresh needs both packages already installed in every pack project. No lockfile was changed.");
	}
	const versions = projects.flatMap((project) => PACKAGES.map((name) => installedVersion(project, name)));
	if (versions.some((version) => version === undefined) || new Set(versions).size !== 1) {
		throw new Error("Installed SDK and CLI versions differ or are missing; game data was not refreshed.");
	}
	const bin = (dependencies.localBin ?? localCliBin)(projects[0] as string);
	const args = [bin, "game-data", root,
		...(options.gamePath ? ["--game-path", options.gamePath] : []),
		...(options.timeout ? ["--timeout", options.timeout] : []),
		...(options.contentTypes ? ["--content-types"] : []),
		...(options.force ? ["--force"] : [])];
	if (run(process.execPath, args, root) !== 0) throw new Error("Local kdf game-data failed; previous installed reflection files remain available.");
	lines.push(`Refreshed game data with local CLI ${versions[0]}.`);
	return { lines, applied: true };
}

function packageManager(root: string): "npm" | "pnpm" {
	const path = join(root, "package.json");
	const manifest = existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) as { packageManager?: string } : {};
	const explicit = manifest.packageManager?.split("@")[0];
	if (explicit && explicit !== "npm" && explicit !== "pnpm") throw new Error(`Unsupported package manager '${explicit}'.`);
	const npmLock = existsSync(join(root, "package-lock.json"));
	const pnpmLock = existsSync(join(root, "pnpm-lock.yaml"));
	if (npmLock && pnpmLock) throw new Error("Both npm and pnpm lockfiles exist; choose one package manager before updating.");
	if (explicit === "npm" && pnpmLock || explicit === "pnpm" && npmLock) throw new Error("packageManager conflicts with root lockfile.");
	return explicit === "npm" || explicit === "pnpm" ? explicit :
		(pnpmLock || existsSync(join(root, "pnpm-workspace.yaml")) ? "pnpm" : "npm");
}

function localVersion(project: string, name: string): string | undefined {
	try {
		const require = createRequire(join(project, "package.json"));
		const path = require.resolve(`${name}/package.json`);
		return (JSON.parse(readFileSync(path, "utf8")) as { version?: string }).version;
	} catch { return undefined; }
}

function localCliBin(project: string): string {
	const require = createRequire(join(project, "package.json"));
	try { return require.resolve("@sf-kmods/kdataforgejs-cli/dist/bin.js"); }
	catch { throw new Error(`Local kdf CLI is missing in ${project}; install both packages first.`); }
}

function runProcess(command: string, args: readonly string[], cwd: string): number {
	const result = spawnSync(command, [...args], { cwd, stdio: "inherit", shell: false });
	if (result.error) throw result.error;
	return result.status ?? 1;
}
