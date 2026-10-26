import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";

export type SupportedPlatform = "win32" | "linux";
export type GameSource = "steam" | "epic";

export interface GameInstallation {
	readonly source: GameSource;
	readonly channel: "stable" | "experimental";
	readonly root: string;
	readonly executable: string;
}

export interface InstallationDetectionOptions {
	readonly platform?: NodeJS.Platform;
	readonly environment?: Readonly<NodeJS.ProcessEnv>;
	readonly steamRoots?: readonly string[];
	readonly epicManifestDirectories?: readonly string[];
}

const STEAM_APP_ID = "526870";
const REQUIRED_MODS = ["KDataForge", "KDataForgeJS"] as const;

export function detectGameInstallations(options: InstallationDetectionOptions = {}): GameInstallation[] {
	const platform = supportedPlatform(options.platform ?? process.platform);
	const environment = options.environment ?? process.env;
	const installations = [
		...detectSteamInstallations(platform, options.steamRoots ?? defaultSteamRoots(platform, environment)),
		...detectEpicInstallations(platform, options.epicManifestDirectories ?? defaultEpicManifestDirectories(platform, environment)),
	];
	const unique = new Map<string, GameInstallation>();
	for (const installation of installations) {
		if (!hasRequiredMods(installation.root)) continue;
		unique.set(installation.root.toLocaleLowerCase(), installation);
	}
	return [...unique.values()].sort(compareInstallations);
}

export function resolveGameInstallation(input: string, platformInput: NodeJS.Platform = process.platform): GameInstallation {
	const platform = supportedPlatform(platformInput);
	const absolute = resolve(input);
	if (!existsSync(absolute)) throw new Error(`Satisfactory path '${absolute}' does not exist.`);
	const inputIsFile = statSync(absolute).isFile();
	const root = inputIsFile ? dirname(absolute) : absolute;
	const executable = resolveExecutable(root, platform, inputIsFile ? absolute : undefined);
	assertRequiredMods(root);
	return {
		source: basename(executable).toLocaleLowerCase().includes("egs") ? "epic" : "steam",
		channel: root.toLocaleLowerCase().includes("experimental") ? "experimental" : "stable",
		root,
		executable,
	};
}

export function supportedPlatform(platform: NodeJS.Platform): SupportedPlatform {
	if (platform === "win32" || platform === "linux") return platform;
	throw new Error(`Game-data export supports Windows and Linux; current platform is '${platform}'.`);
}

function detectSteamInstallations(platform: SupportedPlatform, roots: readonly string[]): GameInstallation[] {
	const installations: GameInstallation[] = [];
	for (const steamRoot of roots) {
		if (!existsSync(steamRoot)) continue;
		const libraries = [steamRoot, ...readSteamLibraryPaths(join(steamRoot, "steamapps/libraryfolders.vdf"))];
		for (const library of libraries) {
			const steamApps = join(library, "steamapps");
			const manifestPath = join(steamApps, `appmanifest_${STEAM_APP_ID}.acf`);
			const manifest = readTextIfPresent(manifestPath);
			const installDirectory = manifest === undefined ? "Satisfactory" : fieldValue(manifest, "installdir") ?? "Satisfactory";
			const root = join(steamApps, "common", installDirectory);
			if (!existsSync(manifestPath) && !existsSync(root)) continue;
			const executable = join(root, "FactoryGameSteam.exe");
			if (!existsSync(executable)) continue;
			installations.push({
				source: "steam",
				channel: fieldValue(manifest ?? "", "betakey")?.toLocaleLowerCase().includes("experimental") ? "experimental" : "stable",
				root,
				executable,
			});
		}
	}
	return installations;
}

function detectEpicInstallations(platform: SupportedPlatform, manifestDirectories: readonly string[]): GameInstallation[] {
	if (platform !== "win32") return [];
	const installations: GameInstallation[] = [];
	for (const directory of manifestDirectories) {
		if (!existsSync(directory)) continue;
		for (const entry of readdirSync(directory, { withFileTypes: true }).slice(0, 1_000)) {
			if (!entry.isFile() || !entry.name.toLocaleLowerCase().endsWith(".item")) continue;
			const file = join(directory, entry.name);
			if (statSync(file).size > 1024 * 1024) continue;
			const manifest = parseJsonRecord(readFileSync(file, "utf8"));
			if (manifest === undefined) continue;
			const displayName = stringField(manifest, "DisplayName");
			const appName = stringField(manifest, "AppName");
			if (!`${displayName} ${appName}`.toLocaleLowerCase().includes("satisfactory")) continue;
			const root = stringField(manifest, "InstallLocation");
			if (root === undefined || !isAbsolute(root)) continue;
			const executable = join(root, "FactoryGameEGS.exe");
			if (!existsSync(executable)) continue;
			installations.push({
				source: "epic",
				channel: `${displayName} ${appName}`.toLocaleLowerCase().includes("experimental") ? "experimental" : "stable",
				root,
				executable,
			});
		}
	}
	return installations;
}

function defaultSteamRoots(platform: SupportedPlatform, environment: Readonly<NodeJS.ProcessEnv>): string[] {
	if (platform === "win32") {
		return [windowsSteamRegistryPath(), environment["ProgramFiles(x86)"], environment.ProgramFiles]
			.filter((value): value is string => value !== undefined)
			.map((value) => basename(value).toLocaleLowerCase() === "steam" ? value : join(value, "Steam"));
	}
	const home = environment.HOME ?? homedir();
	return [join(home, ".steam/steam"), join(home, ".local/share/Steam"), join(home, ".var/app/com.valvesoftware.Steam/.local/share/Steam")];
}

function windowsSteamRegistryPath(): string | undefined {
	try {
		const output = execFileSync("reg.exe", ["query", "HKCU\\Software\\Valve\\Steam", "/v", "SteamPath"], {
			encoding: "utf8",
			stdio: ["ignore", "pipe", "ignore"],
			windowsHide: true,
		});
		return output.match(/SteamPath\s+REG_SZ\s+(.+)$/imu)?.[1]?.trim();
	} catch {
		return undefined;
	}
}

function defaultEpicManifestDirectories(platform: SupportedPlatform, environment: Readonly<NodeJS.ProcessEnv>): string[] {
	if (platform !== "win32") return [];
	return [join(environment.ProgramData ?? "C:\\ProgramData", "Epic/EpicGamesLauncher/Data/Manifests")];
}

function resolveExecutable(root: string, platform: SupportedPlatform, explicit?: string): string {
	if (explicit !== undefined) return explicit;
	for (const name of platform === "win32" ? ["FactoryGameSteam.exe", "FactoryGameEGS.exe"] : ["FactoryGameSteam.exe"]) {
		const executable = join(root, name);
		if (existsSync(executable)) return executable;
	}
	throw new Error(`No supported Satisfactory client executable found in '${root}'.`);
}

function assertRequiredMods(root: string): void {
	for (const mod of REQUIRED_MODS) {
		if (!hasMod(root, mod)) throw new Error(`Satisfactory installation '${root}' does not contain required ${mod} mod.`);
	}
}

function hasRequiredMods(root: string): boolean {
	return REQUIRED_MODS.every((mod) => hasMod(root, mod));
}

function hasMod(root: string, mod: string): boolean {
	return [
		join(root, "FactoryGame/Mods", mod, `${mod}.uplugin`),
		join(root, "FactoryGame/Mods/GameFeatures", mod, `${mod}.uplugin`),
	].some(existsSync);
}

function readSteamLibraryPaths(file: string): string[] {
	const content = readTextIfPresent(file);
	if (content === undefined) return [];
	const paths: string[] = [];
	for (const fields of parseQuotedFields(content)) {
		for (let index = 0; index + 1 < fields.length; index += 1) {
			if (fields[index]?.toLocaleLowerCase() === "path") paths.push(fields[index + 1] as string);
		}
	}
	return paths;
}

function fieldValue(content: string, wanted: string): string | undefined {
	for (const fields of parseQuotedFields(content)) {
		for (let index = 0; index + 1 < fields.length; index += 1) {
			if (fields[index]?.toLocaleLowerCase() === wanted.toLocaleLowerCase()) return fields[index + 1];
		}
	}
	return undefined;
}

function parseQuotedFields(content: string): string[][] {
	return content.split(/\r?\n/u).map((line) => [...line.matchAll(/"((?:\\.|[^"\\])*)"/gu)].map((match) => unescapeVdf(match[1] ?? "")));
}

function unescapeVdf(value: string): string {
	return value.replaceAll("\\\\", "\\").replaceAll('\\"', '"');
}

function readTextIfPresent(path: string): string | undefined {
	return existsSync(path) ? readFileSync(path, "utf8") : undefined;
}

function parseJsonRecord(content: string): Readonly<Record<string, unknown>> | undefined {
	try {
		const value: unknown = JSON.parse(content);
		return typeof value === "object" && value !== null && !Array.isArray(value) ? value as Readonly<Record<string, unknown>> : undefined;
	} catch {
		return undefined;
	}
}

function stringField(record: Readonly<Record<string, unknown>>, key: string): string | undefined {
	const value = record[key];
	return typeof value === "string" ? value : undefined;
}

function compareInstallations(left: GameInstallation, right: GameInstallation): number {
	const sourceOrder: Readonly<Record<GameSource, number>> = { steam: 0, epic: 1 };
	return sourceOrder[left.source] - sourceOrder[right.source] || left.channel.localeCompare(right.channel) || left.root.localeCompare(right.root);
}
