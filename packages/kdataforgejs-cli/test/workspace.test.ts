import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, it } from "node:test";
import { fileURLToPath } from "node:url";
import { run } from "../src/cli.js";
import { checkProject } from "../src/commands/check.js";
import { doctorProject } from "../src/commands/doctor.js";
import { importGameData, inspectGameData } from "../src/commands/gameData.js";
import { initProject } from "../src/commands/init.js";
import { syncProject } from "../src/commands/sync.js";
import { updateProject } from "../src/commands/update.js";
import { discoverPacks } from "../src/pack/discover.js";
import { commitStaged } from "../src/pack/transaction.js";

const roots: string[] = [];
const types = resolve(dirname(fileURLToPath(import.meta.url)), "../../../kdataforgejs-types/tsconfig.base.json");
const io = { out: () => undefined, err: () => undefined };
after(() => { for (const root of roots) rmSync(root, { recursive: true, force: true }); });

function workspace(manager: "npm" | "pnpm" = "npm"): { root: string; alpha: string; beta: string } {
	const root = mkdtempSync(join(tmpdir(), "kdf-workspace-"));
	roots.push(root);
	const alpha = join(root, "packs/Alpha");
	const beta = join(root, "packs/Beta");
	initProject({ directory: alpha, name: "Alpha", force: false });
	initProject({ directory: beta, name: "Beta", force: false });
	for (const project of [alpha, beta]) {
		writeFileSync(join(project, "tsconfig.json"), JSON.stringify({ extends: types, include: ["DataForge/**/*.ts"] }));
	}
	writeFileSync(join(root, "package.json"), JSON.stringify({ private: true, packageManager: `${manager}@10`,
		...(manager === "npm" ? { workspaces: ["packs/*"] } : {}) }));
	if (manager === "pnpm") writeFileSync(join(root, "pnpm-workspace.yaml"), "packages:\n  - packs/*\n");
	return { root, alpha, beta };
}

function file(path: string, content: string): void {
	mkdirSync(dirname(path), { recursive: true });
	writeFileSync(path, content);
}

it("discovers npm and pnpm packs; selects unique refs without running workspace scripts", () => {
	for (const manager of ["npm", "pnpm"] as const) {
		const { root, beta } = workspace(manager);
		assert.deepEqual(discoverPacks(root).packs.map((pack) => pack.ref), ["Alpha", "Beta"]);
		assert.deepEqual(discoverPacks(root, "Beta").packs.map((pack) => pack.projectRoot), [beta]);
		assert.throws(() => discoverPacks(root, "Missing"), /No pack with ref/);
	}
});

it("rejects escaping workspace symlinks, duplicate refs, and unsupported glob patterns", () => {
	const { root, beta } = workspace();
	writeFileSync(join(beta, "DataForge/pack.yml"), readFileSync(join(beta, "DataForge/pack.yml"), "utf8").replace("ref: Beta", "ref: alpha"));
	assert.throws(() => discoverPacks(root), /Duplicate pack ref/);
	writeFileSync(join(root, "package.json"), JSON.stringify({ workspaces: ["packs/**"] }));
	assert.throws(() => discoverPacks(root), /Unsupported workspace pattern/);
	const outside = mkdtempSync(join(tmpdir(), "kdf-outside-"));
	roots.push(outside);
	rmSync(beta, { recursive: true });
	symlinkSync(outside, beta, "dir");
	writeFileSync(join(root, "package.json"), JSON.stringify({ workspaces: ["packs/*"] }));
	assert.throws(() => discoverPacks(root), /No pack.yml|escapes root/);
});

it("routes checks and add to selected pack; doctor never writes generated config types", async () => {
	const { root, alpha, beta } = workspace();
	file(join(beta, "DataForge/scripts/error.ts"), "const wrong: string = 3;\n");
	assert.ok(checkProject(root).some((diagnostic) => diagnostic.code === "TS2322"));
	assert.deepEqual(checkProject(root, undefined, "Alpha").map((diagnostic) => diagnostic.code), []);
	assert.equal(await run(["check", root, "--pack", "Alpha"], io, "0"), 0);
	assert.equal(await run(["check", root], io, "0"), 1);
	assert.equal(await run(["add", "handler", "Extra", root], io, "0"), 2);
	assert.equal(await run(["add", "handler", "Extra", root, "--pack", "Alpha"], io, "0"), 0);
	assert.ok(existsSync(join(alpha, "DataForge/scripts/Extra.ts")));
	assert.ok(!existsSync(join(beta, "DataForge/scripts/Extra.ts")));
	const generated = join(alpha, "DataForge/typescript/kdataforgejs-config.d.ts");
	assert.ok(!existsSync(generated));
	doctorProject(root, undefined, "Alpha");
	assert.ok(!existsSync(generated));
	syncProject(root, "Alpha");
	assert.ok(existsSync(generated));
});

it("keeps generated config keys private to each pack TypeScript project", () => {
	const { root, alpha, beta } = workspace();
	file(join(alpha, "DataForge/alpha.config.yml"), "fields:\n  - {id: alphaOnly, type: boolean}\n");
	syncProject(root);
	const alphaEntry = join(alpha, "DataForge/scripts/main.ts");
	const betaEntry = join(beta, "DataForge/scripts/main.ts");
	for (const entry of [alphaEntry, betaEntry]) {
		writeFileSync(entry, readFileSync(entry, "utf8").replace("console.info",
			'const only: boolean | null = ctx.config.get("alphaOnly");\n\tvoid only;\n\tconsole.info'));
	}
	assert.deepEqual(checkProject(root, undefined, "Alpha").map((item) => item.code), []);
	assert.ok(checkProject(root, undefined, "Beta").some((item) => item.code.startsWith("TS")));
});

it("preflights every manifest before sync writes any pack", () => {
	const { root, alpha, beta } = workspace();
	const alphaManifest = join(alpha, "DataForge/pack.yml");
	const before = readFileSync(alphaManifest, "utf8");
	file(join(alpha, "DataForge/note.txt"), "note");
	const alphaEntry = join(alpha, "DataForge/scripts/main.ts");
	writeFileSync(alphaEntry, readFileSync(alphaEntry, "utf8").replace("console.info",
		'ctx.import("text", "note.txt");\n\tconsole.info'));
	const betaEntry = join(beta, "DataForge/scripts/main.ts");
	writeFileSync(betaEntry, readFileSync(betaEntry, "utf8").replace("console.info",
		'ctx.import("text", input);\n\tconsole.info'));
	assert.throws(() => syncProject(root), /literal kind and path/);
	assert.equal(readFileSync(alphaManifest, "utf8"), before);
	assert.ok(!existsSync(join(alpha, "DataForge/typescript/kdataforgejs-config.d.ts")));
});

it("uses the same writer lock from workspace root and pack project", () => {
	const { root, alpha } = workspace();
	const lock = join(root, ".kdataforge/writer.lock");
	file(lock, "held");
	assert.throws(() => syncProject(alpha), /writer lock exists/);
	rmSync(lock);
	assert.ok(syncProject(alpha).changedTypes.length > 0);
});

it("restores earlier targets when a staged multi-file commit fails", () => {
	const root = mkdtempSync(join(tmpdir(), "kdf-swap-"));
	roots.push(root);
	const first = join(root, "first.txt");
	const second = join(root, "second.txt");
	const stage = join(root, "first.stage");
	file(first, "old-first");
	file(second, "old-second");
	file(stage, "new-first");
	assert.throws(() => commitStaged([{ target: first, stage }, { target: second, stage: join(root, "missing.stage") }]));
	assert.equal(readFileSync(first, "utf8"), "old-first");
	assert.equal(readFileSync(second, "utf8"), "old-second");
});

it("fans one export into two packs and keeps prior files if a later target is unsafe", async () => {
	const { root, alpha, beta } = workspace();
	const installation = join(root, "Satisfactory");
	file(join(installation, "FactoryGameSteam.exe"), "game");
	file(join(installation, "FactoryGame/Mods/GameFeatures/KDataForge/KDataForge.uplugin"), "{}");
	file(join(installation, "FactoryGame/Mods/GameFeatures/KDataForgeJS/KDataForgeJS.uplugin"), "{}");
	const save = join(root, "save.sav");
	file(save, "save");
	let exports = 0;
	const dependencies = { platform: "win32" as const, launch: async (plan: { exportDirectory: string }) => {
		exports += 1;
		file(join(plan.exportDirectory, "export.kdfdata"), `archive-${exports}`);
		file(join(plan.exportDirectory, "typescript/kdataforgejs/kdataforge.generated.d.ts"),
			'declare module "@sf-kmods/kdataforgejs" { export interface Context {} }\n');
		file(join(plan.exportDirectory, "typescript/kdataforgejs/tsconfig.json"), "{}\n");
	}, sleep: async () => undefined };
	const options = { destination: root, projectRoots: [alpha, beta], gamePath: installation, savePath: save };
	await importGameData(options, dependencies);
	assert.equal(exports, 1);
	for (const project of [alpha, beta]) assert.ok(existsSync(join(project, "DataForge/typescript/kdataforgejs/index.d.ts")));
	const archive = join(root, ".kdataforge/satisfactory.kdfdata");
	assert.equal(readFileSync(archive, "utf8"), "archive-1");
	const alphaTypes = join(alpha, "DataForge/typescript/kdataforgejs/index.d.ts");
	const originalTypes = readFileSync(alphaTypes, "utf8");
	writeFileSync(alphaTypes, `${originalTypes}\n// changed\n`);
	assert.equal(inspectGameData(root, [alpha, beta]).status, "stale");
	writeFileSync(alphaTypes, originalTypes);
	const unsafe = join(beta, "DataForge/typescript/kdataforgejs");
	rmSync(unsafe, { recursive: true });
	const outside = mkdtempSync(join(tmpdir(), "kdf-types-outside-"));
	roots.push(outside);
	symlinkSync(outside, unsafe, "dir");
	await assert.rejects(importGameData({ ...options, force: true }, dependencies), /symlink|escape/);
	assert.equal(readFileSync(archive, "utf8"), "archive-1");
	assert.ok(existsSync(join(alpha, "DataForge/typescript/kdataforgejs/index.d.ts")));
});

it("update inspection is read-only; apply uses fixed local package commands and one local game-data invocation", () => {
	const { root } = workspace();
	const calls: Array<{ command: string; args: readonly string[]; cwd: string }> = [];
	const dependencies = { installedVersion: () => "0.1.0", localBin: () => "/local/kdf/bin.js",
		run: (command: string, args: readonly string[], cwd: string) => { calls.push({ command, args, cwd }); return 0; } };
	const inspected = updateProject({ directory: root }, dependencies);
	assert.equal(inspected.applied, false);
	assert.equal(calls.length, 0);
	assert.match(inspected.lines.join("\n"), /Game data stale/);
	updateProject({ directory: root, apply: true }, dependencies);
	assert.deepEqual(calls.map((call) => call.command), ["npm", "npm", process.execPath]);
	assert.ok(calls[0]?.args.includes("--ignore-scripts"));
	assert.deepEqual(calls[2]?.args.slice(0, 3), ["/local/kdf/bin.js", "game-data", root]);
	calls.length = 0;
	updateProject({ directory: root, apply: true, offline: true }, dependencies);
	assert.deepEqual(calls.map((call) => call.command), [process.execPath]);
});

it("update refuses missing offline packages and stops before game-data after package-manager failure", () => {
	const { root } = workspace("pnpm");
	let calls = 0;
	assert.throws(() => updateProject({ directory: root, apply: true, offline: true }, {
		installedVersion: () => undefined, run: () => { calls += 1; return 0; },
	}), /Offline refresh needs both packages/);
	assert.equal(calls, 0);
	assert.throws(() => updateProject({ directory: root, apply: true }, {
		installedVersion: () => "0.1.0", run: () => { calls += 1; return 1; },
	}), /Package-manager update failed/);
	assert.equal(calls, 1);
});

it("pnpm update targets each workspace project without a global install", () => {
	const { root, alpha, beta } = workspace("pnpm");
	const calls: Array<{ command: string; args: readonly string[] }> = [];
	updateProject({ directory: root, apply: true }, { installedVersion: () => "0.1.0",
		localBin: () => "/local/kdf/bin.js",
		run: (command, args) => { calls.push({ command, args }); return 0; } });
	assert.deepEqual(calls.map((call) => call.command), ["pnpm", "pnpm", process.execPath]);
	assert.deepEqual(calls[0]?.args.slice(0, 2), ["--dir", alpha]);
	assert.deepEqual(calls[1]?.args.slice(0, 2), ["--dir", beta]);
	assert.ok(calls.every((call) => !call.args.includes("-g")));
});

it("refuses ambiguous root lockfiles before any update command", () => {
	const { root } = workspace();
	file(join(root, "package-lock.json"), "{}\n");
	file(join(root, "pnpm-lock.yaml"), "lockfileVersion: 9\n");
	let called = false;
	assert.throws(() => updateProject({ directory: root, apply: true }, {
		run: () => { called = true; return 0; },
	}), /Both npm and pnpm lockfiles/);
	assert.equal(called, false);
});
