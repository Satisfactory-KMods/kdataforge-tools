import { parseArgs } from "node:util";
import { checkProject } from "./commands/check.js";
import { addScaffold, type AddKind } from "./commands/add.js";
import { startDev } from "./commands/dev.js";
import { doctorProject } from "./commands/doctor.js";
import { formatProject } from "./commands/format.js";
import { importGameData } from "./commands/gameData.js";
import { hashProject } from "./commands/hash.js";
import { initProject } from "./commands/init.js";
import { lintProject } from "./commands/lint.js";
import { prepareSync, syncProject } from "./commands/sync.js";
import { validateProject } from "./commands/validate.js";
import { updateProject } from "./commands/update.js";
import { formatDiagnostic, hasErrors, type Diagnostic } from "./diagnostics.js";
import { discoverPacks } from "./pack/discover.js";
import { withProjectLock } from "./pack/transaction.js";

const USAGE = `kdf - KDataForgeJS project tooling

Usage:
  kdf init [dir] [--name <PackRef>] [--force]   Create a new KDataForge project with a TypeScript script
  kdf check [dir] [--pack <ref>]                Type-check selected pack projects (no emit)
  kdf format [dir]                              Format DataForge scripts with bundled Biome
  kdf lint [dir]                                Lint DataForge scripts with bundled Biome rules
  kdf validate [dir]                            Check pack.yml, entry, static imports and import digests
  kdf sync [dir]                                Generate imports, hashes and reflection classes from source
  kdf hash [dir]                                Update sha256 of declared script imports in pack.yml
  kdf game-data [dir] [--game-path <path>] [--content-types] [--force]  Export once and refresh every pack
  kdf update [dir] [--apply] [--offline]        Inspect SDK/CLI and game data; apply local update explicitly
  kdf doctor [dir] [--game-path <path>] [--json] Read-only environment and project health
  kdf dev [dir]                                Watch authored files; sync, check, lint, validate
  kdf add <handler|hook|item|recipe|document> <name> [dir] [--pack <ref>] [--type <document-type>]
  kdf --help | --version
`;

export interface Io {
	readonly out: (line: string) => void;
	readonly err: (line: string) => void;
}

export interface CliDependencies {
	readonly importGameData: typeof importGameData;
}

const DEFAULT_DEPENDENCIES: CliDependencies = { importGameData };

/** Runs the CLI and returns the process exit code: 0 ok, 1 findings, 2 usage error. */
export const run = createCli(DEFAULT_DEPENDENCIES);

export function createCli(dependencies: CliDependencies): (argv: readonly string[], io: Io, version: string) => Promise<number> {
	return (argv, io, version) => runCli({ argv, io, version }, dependencies);
}

interface CliRunRequest {
	readonly argv: readonly string[];
	readonly io: Io;
	readonly version: string;
}

async function runCli(request: CliRunRequest, dependencies: CliDependencies): Promise<number> {
	const { argv, io, version } = request;
	let parsed: ReturnType<typeof parseCliArgs>;
	try { parsed = parseCliArgs(argv); } catch (error) { io.err(error instanceof Error ? error.message : String(error)); return 2; }
	const { values, positionals } = parsed;
	const [command, directory = "."] = positionals;
	if (values.version) return print(io.out, version);
	if (values.help || command === undefined) return print(io.out, USAGE);
	try {
		if ((values.apply || values.offline) && command !== "update" ||
			(command === "add" ? positionals.length > 4 : positionals.length > 2)) {
			io.err(`Invalid options or arguments for kdf ${command}.`); return 2;
		}
		if (values.pack && ["init", "game-data", "update"].includes(command)) {
			io.err(`--pack is not supported by kdf ${command}.`); return 2;
		}
		if (values.pack) {
			const target = command === "add" ? positionals[3] ?? "." : directory;
			try { discoverPacks(target, values.pack); }
			catch (error) { io.err(error instanceof Error ? error.message : String(error)); return 2; }
		}
		switch (command) {
			case "init": {
				const result = initProject({ directory, force: values.force, ...(values.name ? { name: values.name } : {}) });
				io.out(`Created KDataForge project '${result.ref}' in ${result.directory}`);
				io.out("Next: npm install, then npm run check");
				return 0;
			}
			case "check":
				return report(io, checkProject(directory, undefined, values.pack), "Type check passed.");
			case "format": {
				const result = formatProject(directory, values.pack);
				io.out(result.changedFiles.length === 0
					? "Files are already formatted."
					: `Formatted files:\n${result.changedFiles.join("\n")}`);
				return 0;
			}
			case "lint":
				return report(io, lintProject(directory, values.pack), "Lint passed.");
			case "validate":
				return report(io, validateProject(directory, values.pack), "Pack is valid.");
			case "sync": {
				const result = syncProject(directory, values.pack);
				io.out(result.changedPacks.length === 0 && result.changedTypes.length === 0
					? "Manifest and config types are already synchronized."
					: `Synchronized manifests and config types:\n${[...result.changedPacks, ...result.changedTypes].join("\n")}`);
				return 0;
			}
			case "hash": {
				const changed = hashProject(directory, values.pack);
				io.out(changed.length === 0 ? "All import digests are up to date." : `Updated digests:\n${changed.join("\n")}`);
				return 0;
			}
			case "game-data": {
				const selection = discoverPacks(directory);
				const projects = [...new Set(selection.packs.map((pack) => pack.projectRoot))];
				const prepared = prepareSync(directory);
				const timeoutMilliseconds = parseTimeout(values.timeout);
				const result = await dependencies.importGameData({
					destination: selection.root,
					projectRoots: projects,
					synchronizedFiles: prepared.files,
					...(values["game-path"] ? { gamePath: values["game-path"] } : {}),
					...(timeoutMilliseconds === undefined ? {} : { timeoutMilliseconds }),
					...(values["content-types"] ? { contentTypes: true } : {}),
					...(values.force ? { force: true } : {}),
				});
				io.out(`${result.cacheHit ? "Reused cached" : "Imported"} game data from ${result.installation.root}`);
				io.out(`Archive: ${result.archivePath}`);
				io.out(`Types: ${result.declarationPath}`);
				return 0;
			}
			case "update": {
				const result = updateProject({ directory, apply: values.apply, offline: values.offline,
					gamePath: values["game-path"], timeout: values.timeout, contentTypes: values["content-types"], force: values.force });
				for (const line of result.lines) io.out(line);
				return 0;
			}
			case "doctor": {
				const checks = doctorProject(directory, values["game-path"], values.pack);
				if (values.json) io.out(JSON.stringify({ checks }, null, 2));
				else for (const check of checks) io.out(`${check.status.toUpperCase()} ${check.name}: ${check.message}`);
				return checks.some((check) => check.status === "fail") ? 1 : 0;
			}
			case "dev":
				startDev(directory, io, values.pack);
				return 0;
			case "add": {
				const [kind, name, root = "."] = positionals.slice(1);
				if (!kind || !name) { io.err(USAGE); return 2; }
				const selection = discoverPacks(root, values.pack);
				if (selection.packs.length !== 1) { io.err("Multiple packs found; pass --pack <ref> for kdf add."); return 2; }
				const path = withProjectLock(selection.root, () => addScaffold(selection.packs[0]?.projectRoot ?? root,
					kind as AddKind, name, values.type, selection.packs[0]?.root));
				io.out(`Created ${path}`);
				return 0;
			}
			default:
				io.err(`Unknown command '${command}'.\n${USAGE}`);
				return 2;
		}
	} catch (error) {
		io.err(error instanceof Error ? error.message : String(error));
		return 1;
	}
}

function parseCliArgs(argv: readonly string[]) {
	return parseArgs({
		args: [...argv],
		allowPositionals: true,
		options: {
			name: { type: "string" },
			pack: { type: "string" },
			"game-path": { type: "string" },
			timeout: { type: "string" },
			"content-types": { type: "boolean", default: false },
			type: { type: "string" },
			json: { type: "boolean", default: false },
			force: { type: "boolean", default: false },
			apply: { type: "boolean", default: false },
			offline: { type: "boolean", default: false },
			help: { type: "boolean", short: "h", default: false },
			version: { type: "boolean", short: "v", default: false },
		},
	});
}

function parseTimeout(value: string | undefined): number | undefined {
	if (value === undefined) return undefined;
	const seconds = Number(value);
	if (!Number.isFinite(seconds) || seconds <= 0) throw new Error("--timeout must be a positive number of seconds.");
	return Math.round(seconds * 1_000);
}

function print(write: (line: string) => void, text: string): number {
	write(text.trimEnd());
	return 0;
}

function report(io: Io, diagnostics: readonly Diagnostic[], success: string): number {
	for (const diagnostic of diagnostics) io.err(formatDiagnostic(diagnostic));
	if (hasErrors(diagnostics)) return 1;
	io.out(success);
	return 0;
}
