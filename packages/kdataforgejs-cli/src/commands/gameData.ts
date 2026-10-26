import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { closeSync, copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, readSync, readdirSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { detectGameInstallations, resolveGameInstallation, supportedPlatform, type GameInstallation, type SupportedPlatform } from "../game/installations.js";
import { formatGeneratedContent } from "../types/format.js";
import { organizeGeneratedDeclarations } from "../types/organize.js";
import { isInside } from "../pack/discover.js";
import { assertSafeWrite, commitStaged, stageText, withProjectLockAsync, type StagedPath } from "../pack/transaction.js";

export interface GameDataOptions {
	readonly destination: string;
	readonly gamePath?: string;
	readonly savePath?: string;
	readonly timeoutMilliseconds?: number;
	readonly contentTypes?: boolean;
	readonly force?: boolean;
	readonly projectRoots?: readonly string[];
	readonly synchronizedFiles?: readonly { readonly path: string; readonly content: string; readonly expected?: string }[];
}

export interface GameLaunchPlan {
	readonly command: string;
	readonly arguments: readonly string[];
	readonly workingDirectory: string;
	readonly exportDirectory: string;
}

export interface GameDataDependencies {
	readonly platform?: NodeJS.Platform;
	readonly launch?: (plan: GameLaunchPlan) => Promise<LaunchedGame | void>;
	readonly sleep?: (milliseconds: number) => Promise<void>;
}

export interface GameExit {
	readonly code: number | null;
	readonly signal: NodeJS.Signals | null;
}

export interface LaunchedGame {
	readonly exited: Promise<GameExit>;
}

export interface GameDataResult {
	readonly installation: GameInstallation;
	readonly archivePath: string;
	readonly declarationPath: string;
	readonly cacheHit?: boolean;
}

export interface GameDataHealth {
	readonly status: "current" | "stale" | "unknown";
	readonly message: string;
}

interface InstalledReceipt {
	readonly schema: 1;
	readonly archiveSha256: string;
	readonly projects: Readonly<Record<string, string>>;
}

const DEFAULT_TIMEOUT_MILLISECONDS = 15 * 60 * 1_000;
const POLL_INTERVAL_MILLISECONDS = 500;
const CACHE_SCHEMA = 1;
const MAX_LOOSE_FILES = 10_000;
const MAX_LOOSE_BYTES = 512 * 1024 * 1024;
function toolVersion(): string {
	let directory = dirname(fileURLToPath(import.meta.url));
	for (let depth = 0; depth < 4; depth += 1) {
		const file = join(directory, "package.json");
		if (existsSync(file)) {
			const packageInfo = JSON.parse(readFileSync(file, "utf8")) as { name?: string; version?: string };
			if (packageInfo.name === "@sf-kmods/kdataforgejs-cli" && packageInfo.version) return packageInfo.version;
		}
		directory = dirname(directory);
	}
	throw new Error("Cannot determine CLI version for game-data cache key.");
}

interface CacheReceipt {
	readonly schema: number;
	readonly files: Record<string, string>;
}

function sha256(path: string): string {
	const hash = createHash("sha256");
	const file = openSync(path, "r");
	const buffer = Buffer.allocUnsafe(1024 * 1024);
	try {
		let count: number;
		while ((count = readSync(file, buffer, 0, buffer.length, null)) > 0) hash.update(buffer.subarray(0, count));
	} finally { closeSync(file); }
	return hash.digest("hex");
}

function filesBelow(root: string): string[] {
	if (!existsSync(root)) return [];
	const files: string[] = [];
	const visit = (directory: string): void => {
		for (const entry of readdirSync(directory, { withFileTypes: true })) {
			const path = join(directory, entry.name);
			if (entry.isDirectory()) visit(path);
			else if (entry.isFile()) files.push(path);
		}
	};
	visit(root);
	return files.sort();
}

function cacheKey(installation: GameInstallation, platform: SupportedPlatform, savePath: string): string | undefined {
	const hash = createHash("sha256");
	hash.update(JSON.stringify({ schema: CACHE_SCHEMA, tool: toolVersion(), platform,
		root: installation.root, source: installation.source, channel: installation.channel }));
	for (const path of [installation.executable, savePath]) hash.update(`${path}\0${sha256(path)}\n`);
	const modFiles = filesBelow(join(installation.root, "FactoryGame/Mods"));
	for (const path of modFiles) {
		if (/\.(uplugin|dll|so|pak|ucas|utoc)$/iu.test(path)) {
			hash.update(`${relative(installation.root, path)}\0${sha256(path)}\n`);
		}
	}
	const factoryGame = join(installation.root, "FactoryGame");
	const looseFiles = [
		...filesBelow(join(factoryGame, "DataForge")),
		...filesBelow(join(factoryGame, "data-editor")),
		...modFiles.filter((path) => /[\\/](?:DataForge|data-editor)[\\/]/iu.test(path)),
	];
	const modPackRoots = modFiles.filter((path) => basename(path).toLowerCase() === "pack.yml").map(dirname);
	for (const path of modFiles) {
		if (modPackRoots.some((packRoot) => path.startsWith(`${packRoot}/`) || path.startsWith(`${packRoot}\\`))) looseFiles.push(path);
	}
	let totalBytes = 0;
	const authoredFiles = [...new Set(looseFiles)].filter((path) =>
		!/(?:^|[\\/])(?:node_modules|\.git)(?:[\\/]|$)/iu.test(path) &&
		!/[\\/]typescript[\\/]kdataforgejs[\\/]/iu.test(path) && !/\.kdfdata$/iu.test(path)).sort();
	if (authoredFiles.length > MAX_LOOSE_FILES) return undefined;
	for (const path of authoredFiles) {
		totalBytes += statSync(path).size;
		if (totalBytes > MAX_LOOSE_BYTES) return undefined;
		hash.update(`${relative(installation.root, path)}\0${sha256(path)}\n`);
	}
	for (const path of filesBelow(join(installation.root, "FactoryGame/Content/Paks"))) {
		const stat = statSync(path);
		hash.update(`${relative(installation.root, path)}\0${stat.size}\0${stat.mtimeMs}\n`);
	}
	return hash.digest("hex");
}

function validCache(path: string): boolean {
	try {
		const receipt = JSON.parse(readFileSync(join(path, "receipt.json"), "utf8")) as CacheReceipt;
		if (receipt.schema !== CACHE_SCHEMA || typeof receipt.files !== "object" || receipt.files === null) return false;
		const expected = Object.keys(receipt.files).sort();
		const actual = filesBelow(path).map((file) => relative(path, file).split("\\").join("/")).filter((file) => file !== "receipt.json").sort();
		if (expected.length !== actual.length || expected.some((file, index) => file !== actual[index])) return false;
		if (!expected.includes("satisfactory.kdfdata") || !expected.includes("typescript/kdataforgejs/kdataforge.generated.d.ts") ||
			!expected.includes("typescript/kdataforgejs/tsconfig.json")) return false;
		return expected.every((file) => sha256(join(path, file)) === receipt.files[file]);
	} catch { return false; }
}

function readCache(cachePath: string): ExportArtifacts | undefined {
	try {
		const version = readFileSync(join(cachePath, "current"), "utf8").trim();
		if (!/^[a-f0-9]{64}$/u.test(version)) return undefined;
		const path = join(cachePath, version);
		return validCache(path) ? {
			archive: join(path, "satisfactory.kdfdata"), typesDirectory: join(path, "typescript/kdataforgejs"),
		} : undefined;
	} catch { return undefined; }
}

/** Read-only comparison with the installed-game cache; unknown never means current. */
export function inspectGameData(destination: string, projectRoots: readonly string[], gamePath?: string): GameDataHealth {
	const root = resolve(destination);
	const archive = join(root, ".kdataforge/satisfactory.kdfdata");
	if (!existsSync(archive)) return { status: "stale", message: "Game-data archive is missing." };
	if (projectRoots.some((project) => !existsSync(join(project, "DataForge/typescript/kdataforgejs/index.d.ts")))) {
		return { status: "stale", message: "One or more pack projects lack generated reflection types." };
	}
	try {
		const receiptPath = join(root, ".kdataforge/install-receipt.json");
		if (!existsSync(receiptPath)) return { status: "unknown", message: "Installed types have no verification receipt; refresh to verify." };
		const receipt = JSON.parse(readFileSync(receiptPath, "utf8")) as InstalledReceipt;
		if (receipt.schema !== 1 || receipt.archiveSha256 !== sha256(archive) ||
			projectRoots.some((project) => receipt.projects[relative(root, project).split("\\").join("/") || "."] !==
				treeHash(join(project, "DataForge/typescript/kdataforgejs")))) {
			return { status: "stale", message: "Installed archive or reflection types differ from the installation receipt." };
		}
		const platform = supportedPlatform(process.platform);
		const installation = selectInstallation(gamePath, platform);
		const savePath = resolveSavePath(undefined, installation, platform);
		const key = cacheKey(installation, platform, savePath);
		const cached = key && readCache(join(root, ".kdataforge/cache", key));
		if (!cached) return { status: "unknown", message: "No validated cache for the current installation and save; refresh to verify." };
		return sha256(archive) === sha256(cached.archive)
			? { status: "current", message: "Archive matches current installed-game cache; reflection types are present." }
			: { status: "stale", message: "Installed archive differs from current installed-game cache." };
	} catch (error) {
		return { status: "unknown", message: error instanceof Error ? error.message : String(error) };
	}
}

function treeHash(root: string): string {
	const hash = createHash("sha256");
	for (const path of filesBelow(root)) hash.update(`${relative(root, path).split("\\").join("/")}\0${sha256(path)}\n`);
	return hash.digest("hex");
}

function publishCache(cachePath: string, artifacts: ExportArtifacts): ExportArtifacts {
	mkdirSync(cachePath, { recursive: true });
	const stage = mkdtempSync(join(cachePath, ".stage-"));
	try {
		copyFileSync(artifacts.archive, join(stage, "satisfactory.kdfdata"));
		mkdirSync(join(stage, "typescript"), { recursive: true });
		cpSync(artifacts.typesDirectory, join(stage, "typescript/kdataforgejs"), { recursive: true });
		const files: Record<string, string> = {};
		for (const file of filesBelow(stage)) files[relative(stage, file).split("\\").join("/")] = sha256(file);
		const receipt = JSON.stringify({ schema: CACHE_SCHEMA, files } satisfies CacheReceipt);
		writeFileSync(join(stage, "receipt.json"), receipt);
		const version = createHash("sha256").update(receipt).digest("hex");
		const versionPath = join(cachePath, version);
		if (existsSync(versionPath) && !validCache(versionPath)) rmSync(versionPath, { recursive: true, force: true });
		if (!existsSync(versionPath)) renameSync(stage, versionPath);
		const pointer = join(cachePath, `.current-${process.pid}-${Date.now()}`);
		try {
			writeFileSync(pointer, `${version}\n`, { flag: "wx" });
			renameSync(pointer, join(cachePath, "current"));
		} finally { rmSync(pointer, { force: true }); }
		return { archive: join(versionPath, "satisfactory.kdfdata"), typesDirectory: join(versionPath, "typescript/kdataforgejs") };
	} finally { rmSync(stage, { recursive: true, force: true }); }
}

export async function importGameData(options: GameDataOptions, dependencies: GameDataDependencies = {}): Promise<GameDataResult> {
	const timeoutMilliseconds = options.timeoutMilliseconds ?? DEFAULT_TIMEOUT_MILLISECONDS;
	assertValidTimeout(timeoutMilliseconds);
	const platform = supportedPlatform(dependencies.platform ?? process.platform);
	const installation = selectInstallation(options.gamePath, platform);
	const savePath = resolveSavePath(options.savePath, installation, platform);
	const destination = resolve(options.destination);
	mkdirSync(destination, { recursive: true });
	return withProjectLockAsync(destination, async () => importLocked({ ...options, destination }, dependencies, platform, installation, savePath, timeoutMilliseconds));
}

async function importLocked(options: GameDataOptions, dependencies: GameDataDependencies, platform: SupportedPlatform,
	installation: GameInstallation, savePath: string, timeoutMilliseconds: number): Promise<GameDataResult> {
	const destination = options.destination;
	const cacheRoot = join(destination, ".kdataforge/cache");
	assertSafeWrite(destination, join(cacheRoot, "current"));
	const key = cacheKey(installation, platform, savePath);
	const cachePath = key === undefined ? undefined : join(cacheRoot, key);
	const cached = options.force || cachePath === undefined ? undefined : readCache(cachePath);
	if (cached) return { ...installExport(options, installation, cached), cacheHit: true };
	const exportDirectory = mkdtempSync(join(tmpdir(), "kdf-game-data-export-"));
	try {
		const plan = createLaunchPlan(installation, platform, exportDirectory, savePath);
		const launchedGame = await (dependencies.launch ?? launchGame)(plan);
		const artifacts = await waitForExport(
			exportDirectory,
			timeoutMilliseconds,
			dependencies.sleep ?? delay,
			launchedGame?.exited,
		);
		if (launchedGame !== undefined) await waitForCleanGameExit(launchedGame.exited);
		const published = cachePath === undefined ? artifacts : publishCache(cachePath, artifacts);
		return { ...installExport(options, installation, published), cacheHit: false };
	} finally {
		cleanupTemporaryExport(exportDirectory);
	}
}

async function waitForCleanGameExit(exited: Promise<GameExit>): Promise<void> {
	await new Promise<void>((resolveExit, rejectExit) => {
		const timer = setTimeout(() => rejectExit(new Error("Game did not exit within 30 seconds after reflection export completed.")), 30_000);
		exited.then((result) => {
			clearTimeout(timer);
			if (result.code !== null && result.code !== 0) rejectExit(new Error(`Game exited with code ${result.code} after reflection export completed.`));
			else resolveExit();
		}, rejectExit);
	});
}

function cleanupTemporaryExport(exportDirectory: string): void {
	rmSync(exportDirectory, { recursive: true, force: true });
	const parent = resolve(exportDirectory, "..");
	const stagingPrefix = `.${basename(exportDirectory)}.kdf-export-`;
	for (const entry of readdirSync(parent, { withFileTypes: true })) {
		if (entry.isDirectory() && entry.name.startsWith(stagingPrefix) && entry.name.endsWith(".stage")) {
			rmSync(join(parent, entry.name), { recursive: true, force: true });
		}
	}
}

export function createLaunchPlan(installation: GameInstallation, platform: SupportedPlatform, exportDirectory: string, savePath?: string): GameLaunchPlan {
	const gameArguments = [
		"-EpicPortal",
		"-log",
		"-nosplash",
		`-KDFReflectionAutoExport=${exportDirectory}`,
		"-KDFReflectionAutoExit",
	];
	if (savePath !== undefined) gameArguments.push(createSaveGameOverride(savePath));
	if (platform === "win32") {
		return { command: installation.executable, arguments: gameArguments, workingDirectory: installation.root, exportDirectory };
	}
	if (installation.root.includes("/.var/app/com.valvesoftware.Steam/")) {
		return {
			command: "flatpak",
			arguments: ["run", "com.valvesoftware.Steam", "-applaunch", "526870", "--", ...gameArguments],
			workingDirectory: installation.root,
			exportDirectory,
		};
	}
	return {
		command: "steam",
		arguments: ["-applaunch", "526870", "--", ...gameArguments],
		workingDirectory: installation.root,
		exportDirectory,
	};
}

function selectInstallation(gamePath: string | undefined, platform: SupportedPlatform): GameInstallation {
	if (gamePath !== undefined) return resolveGameInstallation(gamePath, platform);
	const installations = detectGameInstallations({ platform });
	const installation = installations[0];
	if (installation !== undefined) return installation;
	throw new Error("No Satisfactory installation containing KDataForge and KDataForgeJS was found. Pass --game-path <root-or-executable>.");
}

async function launchGame(plan: GameLaunchPlan): Promise<LaunchedGame> {
	return new Promise<LaunchedGame>((resolveLaunch, rejectLaunch) => {
		const child = spawn(plan.command, [...plan.arguments], {
			cwd: plan.workingDirectory,
			stdio: "inherit",
			windowsHide: false,
			detached: false,
		});
		const exited = new Promise<GameExit>((resolveExit) => {
			child.once("exit", (code, signal) => resolveExit({ code, signal }));
		});
		child.once("spawn", () => resolveLaunch({ exited }));
		child.once("error", rejectLaunch);
	});
}

interface ExportArtifacts {
	readonly archive: string;
	readonly typesDirectory: string;
}

async function waitForExport(
	exportDirectory: string,
	timeoutMilliseconds: number,
	sleep: (milliseconds: number) => Promise<void>,
	exited?: Promise<GameExit>,
): Promise<ExportArtifacts> {
	assertValidTimeout(timeoutMilliseconds);
	const deadline = Date.now() + timeoutMilliseconds;
	const exitState: { value?: GameExit } = {};
	const observeExit = exited?.then((value) => {
		exitState.value = value;
	});
	while (Date.now() <= deadline) {
		const artifacts = findCompletedExport(exportDirectory);
		if (artifacts !== undefined) return artifacts;
		if (exitState.value !== undefined) {
			const reason = exitState.value.code === null ? `signal ${exitState.value.signal ?? "unknown"}` : `code ${exitState.value.code}`;
			throw new Error(`Game exited before reflection export completed (${reason}).`);
		}
		await (observeExit === undefined ? sleep(POLL_INTERVAL_MILLISECONDS) : Promise.race([sleep(POLL_INTERVAL_MILLISECONDS), observeExit]));
	}
	throw new Error(`Timed out after ${timeoutMilliseconds} ms waiting for KDataForge reflection export.`);
}

function createSaveGameOverride(savePath: string): string {
	const saveName = basename(savePath, extname(savePath));
	return `-ini:Engine:[/Script/EngineSettings.GameMapsSettings]:GameDefaultMap=/Game/FactoryGame/Map/GameLevel01/Persistent_Level.Persistent_Level,[/Script/EngineSettings.GameMapsSettings]:LocalMapOptions=?skipOnboarding?listen?loadgame=${saveName}`;
}

function resolveSavePath(explicit: string | undefined, installation: GameInstallation, platform: SupportedPlatform): string {
	if (explicit !== undefined) {
		const path = resolve(explicit);
		if (!existsSync(path) || !statSync(path).isFile()) throw new Error(`Satisfactory save '${path}' does not exist.`);
		return path;
	}
	const roots = platform === "win32"
		? [join(process.env.LOCALAPPDATA ?? join(homedir(), "AppData/Local"), "FactoryGame/Saved/SaveGames")]
		: [
			resolve(installation.root, "../../compatdata/526870/pfx/drive_c/users/steamuser/AppData/Local/FactoryGame/Saved/SaveGames"),
			join(homedir(), ".local/share/Steam/steamapps/compatdata/526870/pfx/drive_c/users/steamuser/AppData/Local/FactoryGame/Saved/SaveGames"),
		];
	const saves = roots.flatMap(findSaveFiles).sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs);
	const latest = saves[0];
	if (latest === undefined) throw new Error("No Satisfactory save was found; start a save once before importing game data.");
	return latest;
}

function findSaveFiles(root: string): string[] {
	if (!existsSync(root)) return [];
	const saves: string[] = [];
	const pending = [root];
	while (pending.length > 0) {
		const directory = pending.pop();
		if (directory === undefined) break;
		for (const entry of readdirSync(directory, { withFileTypes: true })) {
			const path = join(directory, entry.name);
			if (entry.isDirectory()) pending.push(path);
			else if (entry.isFile() && entry.name.toLocaleLowerCase().endsWith(".sav")) saves.push(path);
		}
	}
	return saves;
}

function assertValidTimeout(timeoutMilliseconds: number): void {
	if (!Number.isSafeInteger(timeoutMilliseconds) || timeoutMilliseconds <= 0) {
		throw new Error("Game-data timeout must be a positive integer number of milliseconds.");
	}
}

function findCompletedExport(exportDirectory: string): ExportArtifacts | undefined {
	const archive = readdirSync(exportDirectory, { withFileTypes: true })
		.filter((entry) => entry.isFile() && entry.name.toLocaleLowerCase().endsWith(".kdfdata"))
		.map((entry) => join(exportDirectory, entry.name))
		.sort()
		.at(-1);
	const typesDirectory = join(exportDirectory, "typescript/kdataforgejs");
	if (archive === undefined || !existsSync(join(typesDirectory, "kdataforge.generated.d.ts")) || !existsSync(join(typesDirectory, "tsconfig.json"))) {
		return undefined;
	}
	return { archive, typesDirectory };
}

function installExport(options: GameDataOptions, installation: GameInstallation, artifacts: ExportArtifacts): GameDataResult {
	const destination = resolve(options.destination);
	mkdirSync(destination, { recursive: true });
	const archiveDirectory = join(destination, ".kdataforge");
	mkdirSync(archiveDirectory, { recursive: true });
	const archivePath = join(archiveDirectory, "satisfactory.kdfdata");
	assertSafeWrite(destination, archivePath);
	const projects = [...new Set(options.projectRoots ?? [destination])];
	const stages: StagedPath[] = [];
	try {
		const archiveStage = `${archivePath}.${process.pid}-${Date.now()}.stage`;
		copyFileSync(artifacts.archive, archiveStage);
		stages.push({ target: archivePath, stage: archiveStage });
		let declarationPath = "";
		const projectHashes: Record<string, string> = {};
		for (const project of projects) {
			const root = resolve(project);
			if (!isInside(destination, root) || !isInside(realpathSync(destination), realpathSync(root)) ||
				(options.projectRoots && !existsSync(join(root, "DataForge/pack.yml")))) {
				throw new Error(`Game-data target is not a pack project inside ${destination}: ${root}`);
			}
			const typesDirectory = join(root, "DataForge/typescript/kdataforgejs");
			assertSafeWrite(root, typesDirectory);
			if (existsSync(typesDirectory) && !isInside(root, realpathSync(typesDirectory))) {
				throw new Error(`Generated types escape pack project: ${typesDirectory}`);
			}
			mkdirSync(dirname(typesDirectory), { recursive: true });
			const typesStage = mkdtempSync(join(dirname(typesDirectory), ".kdf-types-"));
			stages.push({ target: typesDirectory, stage: typesStage });
			cpSync(artifacts.typesDirectory, typesStage, { recursive: true });
			organizeGeneratedDeclarations(typesStage, { contentTypes: options.contentTypes });
			projectHashes[relative(destination, root).split("\\").join("/") || "."] = treeHash(typesStage);
			declarationPath ||= join(typesDirectory, "index.d.ts");
			const tsconfigPath = join(root, "tsconfig.kdf.json");
			assertSafeWrite(root, tsconfigPath);
			stages.push(stageText(tsconfigPath, formatGeneratedContent(JSON.stringify({
				extends: "./DataForge/typescript/kdataforgejs/tsconfig.json",
				include: ["DataForge/**/*.ts"],
			}), tsconfigPath)));
		}
		const gitIgnore = join(destination, ".gitignore");
		assertSafeWrite(destination, gitIgnore);
		const existing = existsSync(gitIgnore) ? readFileSync(gitIgnore, "utf8") : "";
		if (!existing.split(/\r?\n/u).includes(".kdataforge/")) {
			const separator = existing.length === 0 || existing.endsWith("\n") ? "" : "\n";
			stages.push(stageText(gitIgnore, `${existing}${separator}.kdataforge/\n`));
		}
		for (const file of options.synchronizedFiles ?? []) {
			assertSafeWrite(destination, file.path);
			const actual = existsSync(file.path) ? readFileSync(file.path, "utf8") : undefined;
			if (actual !== file.expected) throw new Error(`Authored file changed during game-data export: ${file.path}`);
			stages.push(stageText(file.path, file.content));
		}
		const receiptPath = join(archiveDirectory, "install-receipt.json");
		assertSafeWrite(destination, receiptPath);
		stages.push(stageText(receiptPath, `${JSON.stringify({ schema: 1, archiveSha256: sha256(artifacts.archive),
			projects: projectHashes } satisfies InstalledReceipt)}\n`));
		commitStaged(stages);
		return { installation, archivePath, declarationPath };
	} finally {
		for (const stage of stages) rmSync(stage.stage, { recursive: true, force: true });
	}
}

function delay(milliseconds: number): Promise<void> {
	return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}
