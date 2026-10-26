import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { after, describe, it } from "node:test";
import { createLaunchPlan, importGameData } from "../src/commands/gameData.js";
import { detectGameInstallations, resolveGameInstallation, supportedPlatform } from "../src/game/installations.js";

const workdirs: string[] = [];

function temporaryDirectory(): string {
	const directory = mkdtempSync(join(tmpdir(), "kdf-game-data-"));
	workdirs.push(directory);
	return directory;
}

function writeFile(path: string, content = "fixture"): void {
	mkdirSync(join(path, ".."), { recursive: true });
	writeFileSync(path, content);
}

function createWindowsInstallation(root: string, executable = "FactoryGameSteam.exe"): void {
	writeFile(join(root, executable));
	writeFile(join(root, "FactoryGame/Mods/GameFeatures/KDataForge/KDataForge.uplugin"), "{}");
	writeFile(join(root, "FactoryGame/Mods/GameFeatures/KDataForgeJS/KDataForgeJS.uplugin"), "{}");
}

after(() => {
	for (const directory of workdirs) rmSync(directory, { recursive: true, force: true });
});

describe("game installation detection", () => {
	it("finds a modded Satisfactory install in a custom Steam library", () => {
		const steam = join(temporaryDirectory(), "Steam");
		const library = join(temporaryDirectory(), "SteamLibrary");
		writeFile(join(steam, "steamapps/libraryfolders.vdf"), `"libraryfolders"\n{\n "1" { "path" "${library.replaceAll("\\", "\\\\")}" }\n}`);
		writeFile(join(library, "steamapps/appmanifest_526870.acf"), '"AppState"\n{\n "installdir" "Satisfactory"\n}');
		createWindowsInstallation(join(library, "steamapps/common/Satisfactory"));

		const installations = detectGameInstallations({
			platform: "win32",
			environment: { "ProgramFiles(x86)": join(steam, "..") },
			steamRoots: [steam],
			epicManifestDirectories: [],
		});

		assert.equal(installations.length, 1);
		assert.equal(installations[0]?.source, "steam");
		assert.equal(installations[0]?.root, join(library, "steamapps/common/Satisfactory"));
	});

	it("ignores installations without both KDataForge runtime mods", () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		writeFile(join(root, "FactoryGameSteam.exe"));
		writeFile(join(root, "FactoryGame/Mods/GameFeatures/KDataForge/KDataForge.uplugin"), "{}");

		assert.throws(() => resolveGameInstallation(root, "win32"), /KDataForgeJS/);
	});

	it("accepts an explicit game executable path", () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		createWindowsInstallation(root, "FactoryGameEGS.exe");

		const installation = resolveGameInstallation(join(root, "FactoryGameEGS.exe"), "win32");

		assert.equal(installation.source, "epic");
		assert.equal(installation.executable, join(root, "FactoryGameEGS.exe"));
	});

	it("finds a modded Epic installation from its launcher manifest", () => {
		const root = join(temporaryDirectory(), "SatisfactoryExperimental");
		createWindowsInstallation(root, "FactoryGameEGS.exe");
		const manifests = join(temporaryDirectory(), "Manifests");
		writeFile(join(manifests, "satisfactory.item"), JSON.stringify({
			DisplayName: "Satisfactory Experimental",
			AppName: "CrabTest",
			InstallLocation: root,
		}));

		const installations = detectGameInstallations({
			platform: "win32",
			steamRoots: [],
			epicManifestDirectories: [manifests],
		});

		assert.equal(installations[0]?.source, "epic");
		assert.equal(installations[0]?.channel, "experimental");
	});

	it("reports unsupported platforms and missing explicit paths", () => {
		assert.throws(() => supportedPlatform("darwin"), /Windows and Linux/);
		assert.throws(() => resolveGameInstallation(join(temporaryDirectory(), "missing"), "win32"), /does not exist/);
	});
});

describe("game-data import", () => {
	it("launches Windows game with automatic export and imports archive plus generated types", async () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		createWindowsInstallation(root);
		const destination = join(temporaryDirectory(), "Project");
		mkdirSync(destination, { recursive: true });
		writeFile(join(destination, "DataForge/typescript/kdataforgejs/paths/stale.d.ts"), "declare const stale: true;\n");
		const savePath = join(temporaryDirectory(), "My Save.sav");
		writeFile(savePath);
		let launchedCommand = "";
		let launchedArguments: readonly string[] = [];
		let retainedStagingDirectory = "";

		const result = await importGameData(
			{ destination, gamePath: root, savePath, timeoutMilliseconds: 1_000 },
			{
				platform: "win32",
				launch: async (plan) => {
					launchedCommand = plan.command;
					launchedArguments = plan.arguments;
					retainedStagingDirectory = resolve(plan.exportDirectory, "..", `.${basename(plan.exportDirectory)}.kdf-export-fixture.stage`);
					mkdirSync(retainedStagingDirectory, { recursive: true });
					writeFile(join(plan.exportDirectory, "2026-10-25T12-00-00.000Z.kdfdata"), "archive");
					writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/kdataforge.generated.d.ts"), `declare module "@sf-kmods/kdataforgejs" {
  export interface Context {}
	interface NativeClassPathMap {
		readonly "/Game/FactoryGame/Alpha.BP_Alpha_C": import("satisfactory").Alpha;
		readonly "/Script/FactoryGame.Beta": import("satisfactory").Beta;
	}
}
interface Console { log(value: unknown): void; }
declare const console: Console;
declare module "satisfactory" {
  import type { UObject } from "@sf-kmods/kdataforgejs";
  export interface Alpha extends UObject {}
  export const Alpha: unknown;
  export interface Beta extends UObject {}
  export const Beta: unknown;
}
`);
					writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/tsconfig.json"), "{}\n");
					writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/paths.json"), "{}\n");
					return { exited: Promise.resolve({ code: 0, signal: null }) };
				},
				sleep: async () => undefined,
			},
		);

		assert.equal(launchedCommand, join(root, "FactoryGameSteam.exe"));
		assert.ok(launchedArguments.some((argument) => argument.startsWith("-KDFReflectionAutoExport=")));
		assert.ok(launchedArguments.includes("-KDFReflectionAutoExit"));
		assert.ok(launchedArguments.some((argument) => argument.includes("loadgame=My Save")));
		assert.equal(readFileSync(join(destination, ".kdataforge/satisfactory.kdfdata"), "utf8"), "archive");
		const installedTypes = join(destination, "DataForge/typescript/kdataforgejs");
		assert.equal(existsSync(join(installedTypes, "kdataforge.generated.d.ts")), false);
		assert.equal(existsSync(join(installedTypes, "paths/stale.d.ts")), false);
		const declarationIndex = readFileSync(join(installedTypes, "index.d.ts"), "utf8");
		assert.match(declarationIndex, /reference path/);
		assert.match(declarationIndex, /declare module "@sf-kmods\/kdataforgejs" \{\}/);
		assert.match(declarationIndex, /declare module "satisfactory" \{\}/);
		assert.match(readFileSync(join(installedTypes, "runtime.d.ts"), "utf8"), /interface Context/);
		assert.match(readFileSync(join(installedTypes, "globals.d.ts"), "utf8"), /declare const console/);
		assert.match(readFileSync(join(installedTypes, "satisfactory/a.d.ts"), "utf8"), /interface Alpha/);
		assert.match(readFileSync(join(installedTypes, "satisfactory/b.d.ts"), "utf8"), /interface Beta/);
		assert.match(
			readFileSync(join(installedTypes, "paths/game-factorygame.d.ts"), "utf8"),
			/import type \* as Satisfactory from "satisfactory";[\s\S]*Satisfactory\.Alpha/,
		);
		assert.ok(existsSync(join(installedTypes, "paths/script-factorygame.d.ts")));
		const declarations = readdirSync(installedTypes, { recursive: true, withFileTypes: true })
			.filter((entry) => entry.isFile() && entry.name.endsWith(".d.ts"))
			.map((entry) => readFileSync(join(entry.parentPath, entry.name), "utf8"))
			.join("\n");
		assert.doesNotMatch(declarations, /import\(["']satisfactory["']\)\./u);
		assert.match(readFileSync(join(destination, "tsconfig.kdf.json"), "utf8"), /DataForge\/typescript\/kdataforgejs\/tsconfig.json/);
		assert.match(readFileSync(join(destination, ".gitignore"), "utf8"), /^\.kdataforge\/$/m);
		assert.equal(result.archivePath, join(destination, ".kdataforge/satisfactory.kdfdata"));
		assert.equal(result.declarationPath, join(installedTypes, "index.d.ts"));
		assert.equal(existsSync(retainedStagingDirectory), false);
	});

	it("uses Steam launcher on Linux", async () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		writeFile(join(root, "FactoryGameSteam.exe"));
		writeFile(join(root, "FactoryGame/Mods/GameFeatures/KDataForge/KDataForge.uplugin"), "{}");
		writeFile(join(root, "FactoryGame/Mods/GameFeatures/KDataForgeJS/KDataForgeJS.uplugin"), "{}");
		const destination = join(temporaryDirectory(), "Project");
		const savePath = join(temporaryDirectory(), "Linux Save.sav");
		writeFile(savePath);
		let command = "";

		await importGameData(
			{ destination, gamePath: root, savePath, timeoutMilliseconds: 1_000 },
			{
				platform: "linux",
				launch: async (plan) => {
					command = plan.command;
					writeFile(join(plan.exportDirectory, "export.kdfdata"));
					writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/kdataforge.generated.d.ts"));
					writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/tsconfig.json"), "{}");
				},
				sleep: async () => undefined,
			},
		);

		assert.equal(command, "steam");
	});

	it("reuses a validated project cache, rebuilds corrupt entries, and honors --force", async () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		createWindowsInstallation(root);
		const savePath = join(temporaryDirectory(), "Test.sav");
		writeFile(savePath);
		const destination = temporaryDirectory();
		let launches = 0;
		const dependencies = {
			platform: "win32" as const,
			launch: async (plan: { exportDirectory: string }) => {
				launches += 1;
				writeFile(join(plan.exportDirectory, "export.kdfdata"), `archive-${launches}`);
				writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/kdataforge.generated.d.ts"), 'declare module "@sf-kmods/kdataforgejs" { export interface SchematicDocument { readonly type: "schematic"; } }\n');
				writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/tsconfig.json"), "{}");
			},
			sleep: async () => undefined,
		};
		const options = { destination, gamePath: root, savePath };
		const first = await importGameData(options, dependencies);
		assert.equal(first.cacheHit, false);
		const cache = join(destination, ".kdataforge/cache");
		const key = join(cache, readdirSync(cache)[0] as string);
		const entry = join(key, readFileSync(join(key, "current"), "utf8").trim());
		assert.ok(existsSync(join(entry, "receipt.json")));
		assert.equal((await importGameData(options, dependencies)).cacheHit, true);
		assert.equal(launches, 1);
		const runtimeTypes = join(destination, "DataForge/typescript/kdataforgejs/runtime.d.ts");
		assert.equal((await importGameData({ ...options, contentTypes: true }, dependencies)).cacheHit, true);
		assert.match(readFileSync(runtimeTypes, "utf8"), /KDFSchema_schematic___defs_entry/);
		assert.equal((await importGameData(options, dependencies)).cacheHit, true);
		assert.doesNotMatch(readFileSync(runtimeTypes, "utf8"), /KDFSchema_schematic___defs_entry/);
		assert.equal(launches, 1);
		writeFileSync(join(entry, "satisfactory.kdfdata"), "corrupt");
		assert.equal((await importGameData(options, dependencies)).cacheHit, false);
		assert.equal(launches, 2);
		assert.equal(readFileSync(first.archivePath, "utf8"), "archive-2");
		await importGameData({ ...options, force: true, contentTypes: true }, dependencies);
		assert.match(readFileSync(runtimeTypes, "utf8"), /KDFSchema_schematic___defs_entry/);
		assert.equal(launches, 3);
	});

	it("invalidates cache when loose YAML, TypeScript, or imported pack content changes", async () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		createWindowsInstallation(root);
		const pack = join(root, "FactoryGame/DataForge/Demo");
		const manifest = join(pack, "pack.yml");
		const script = join(pack, "scripts/main.ts");
		const imported = join(root, "FactoryGame/Mods/GameFeatures/KDataForgeJS/DataForge/Addon/config.json");
		writeFile(manifest, "ref: Demo\n");
		writeFile(script, "export default 1;\n");
		writeFile(imported, "{\"value\":1}\n");
		const savePath = join(temporaryDirectory(), "Test.sav");
		writeFile(savePath);
		const options = { destination: temporaryDirectory(), gamePath: root, savePath };
		let launches = 0;
		const dependencies = {
			platform: "win32" as const,
			launch: async (plan: { exportDirectory: string }) => {
				launches += 1;
				writeFile(join(plan.exportDirectory, "export.kdfdata"), `archive-${launches}`);
				writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/kdataforge.generated.d.ts"));
				writeFile(join(plan.exportDirectory, "typescript/kdataforgejs/tsconfig.json"), "{}");
			},
			sleep: async () => undefined,
		};
		assert.equal((await importGameData(options, dependencies)).cacheHit, false);
		assert.equal((await importGameData(options, dependencies)).cacheHit, true);
		writeFileSync(manifest, "ref: Changed\n");
		assert.equal((await importGameData(options, dependencies)).cacheHit, false);
		writeFileSync(script, "export default 2;\n");
		assert.equal((await importGameData(options, dependencies)).cacheHit, false);
		writeFileSync(imported, "{\"value\":2}\n");
		assert.equal((await importGameData(options, dependencies)).cacheHit, false);
		assert.equal(launches, 4);
	});

	it("uses Flatpak for a Flatpak Steam library", () => {
		const root = "/home/test/.var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/Satisfactory";
		const plan = createLaunchPlan(
			{ source: "steam", channel: "stable", root, executable: `${root}/FactoryGameSteam.exe` },
			"linux",
			"/tmp/export",
		);
		assert.equal(plan.command, "flatpak");
		assert.deepEqual(plan.arguments.slice(0, 4), ["run", "com.valvesoftware.Steam", "-applaunch", "526870"]);
	});

	it("rejects an invalid timeout before polling forever", async () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		createWindowsInstallation(root);
		let launched = false;
		await assert.rejects(
			importGameData(
				{ destination: temporaryDirectory(), gamePath: root, timeoutMilliseconds: 0 },
				{
					platform: "win32",
					launch: async () => {
						launched = true;
					},
					sleep: async () => undefined,
				},
			),
			/positive integer/,
		);
		assert.equal(launched, false);
	});

	it("reports a game exit before an export instead of waiting for timeout", async () => {
		const root = join(temporaryDirectory(), "Satisfactory");
		createWindowsInstallation(root);
		const savePath = join(temporaryDirectory(), "Test.sav");
		writeFile(savePath);
		await assert.rejects(
			importGameData(
				{
					destination: temporaryDirectory(),
					gamePath: root,
					savePath,
					timeoutMilliseconds: 60_000,
				},
				{
					platform: "win32",
					launch: async () => ({ exited: Promise.resolve({ code: 1, signal: null }) }),
					sleep: async () => undefined,
				},
			),
			/exited before reflection export.*code 1/i,
		);
	});
});
