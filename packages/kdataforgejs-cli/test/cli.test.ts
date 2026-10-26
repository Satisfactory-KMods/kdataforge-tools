import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { after, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { createCli, run } from "../src/cli.js";
import { checkProject } from "../src/commands/check.js";
import { initProject, packRefFrom } from "../src/commands/init.js";
import { syncProject } from "../src/commands/sync.js";
import { validateProject } from "../src/commands/validate.js";
import { sha256Of } from "../src/pack/imports.js";

const TYPES_BASE = resolve(dirname(fileURLToPath(import.meta.url)), "../../../kdataforgejs-types/tsconfig.base.json");
const workdirs: string[] = [];

function newProject(name = "DemoPack"): string {
	const root = mkdtempSync(join(tmpdir(), "kdf-cli-"));
	workdirs.push(root);
	const directory = join(root, name);
	initProject({ directory, force: false });
	return directory;
}

function codes(directory: string): string[] {
	return validateProject(directory).map((d) => d.code);
}

function write(directory: string, path: string, content: string): void {
	const file = join(directory, path);
	mkdirSync(dirname(file), { recursive: true });
	writeFileSync(file, content);
}

after(() => {
	for (const dir of workdirs) rmSync(dir, { recursive: true, force: true });
});

describe("names", () => {
	it("derives a safe pack ref", () => {
		assert.equal(packRefFrom("1 my pack!"), "my-pack");
	});
});

describe("init", () => {
	it("creates a project that validates", () => {
		assert.deepEqual(codes(newProject()), []);
	});
	it("writes ref without an authored script id", () => {
		const pack = readFileSync(join(newProject("MyPack"), "DataForge/pack.yml"), "utf8");
		assert.match(pack, /^ref: MyPack$/m);
		assert.doesNotMatch(pack, /^\s+id:/m);
	});
	it("renames _gitignore", () => {
		assert.match(readFileSync(join(newProject(), ".gitignore"), "utf8"), /node_modules/);
	});
	it("makes manifest generation part of normal verification", () => {
		const project = JSON.parse(readFileSync(join(newProject(), "package.json"), "utf8")) as {
			readonly scripts: Readonly<Record<string, string>>;
		};
		assert.equal(project.scripts.sync, "kdf sync");
		assert.equal(project.scripts.format, "kdf format");
		assert.equal(project.scripts.lint, "kdf lint");
		assert.equal(project.scripts.refresh, "kdf game-data");
		assert.equal(project.scripts.fix, "kdf format && kdf sync");
		assert.equal(project.scripts.test, "npm run verify");
		assert.equal(project.scripts.verify, "kdf sync && kdf check && kdf lint && kdf validate");
	});
	it("refuses a non-empty directory without --force", () => {
		const directory = newProject();
		assert.throws(() => initProject({ directory, force: false }), /not empty/);
	});
});

describe("validate", () => {
	it("rejects dynamic import", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/lib.ts", "export const load = () => import('./other.ts');\n");
		assert.ok(codes(directory).includes("source.dynamic-import"));
	});
	it("rejects external packages", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/lib.ts", "import 'lodash';\nexport const a = 1;\n");
		assert.ok(codes(directory).includes("source.external-import"));
	});
	it("accepts the generated Satisfactory reflection module", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/lib.ts", "import { FGItemDescriptor } from 'satisfactory';\nexport const item = FGItemDescriptor;\n");
		assert.deepEqual(codes(directory), []);
	});
	it("rejects imports that leave the scripts folder", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/lib.ts", "import '../../outside.ts';\nexport const a = 1;\n");
		assert.ok(codes(directory).includes("source.outside-scripts"));
	});
	it("accepts explicit .ts and extensionless imports like the runtime", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/main.ts", readFileSync(join(directory, "DataForge/scripts/main.ts"), "utf8").replace("./lib\"", "./lib.ts\""));
		assert.deepEqual(codes(directory), []);
	});
	it("rejects a package.json inside the pack", () => {
		const directory = newProject();
		write(directory, "DataForge/package.json", "{}");
		assert.ok(codes(directory).includes("pack.forbidden"));
	});
	it("rejects a non-canonical entry", () => {
		const directory = newProject();
		const file = join(directory, "DataForge/pack.yml");
		writeFileSync(file, readFileSync(file, "utf8").replace("scripts/main.ts", "Scripts/main.ts"));
		assert.ok(codes(directory).includes("script.entry"));
	});
	it("requires only a default handler", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/main.ts", "export const x = 1;\n");
		assert.deepEqual(codes(directory), ["entry.handler"]);
	});
	it("rejects manifest capabilities", () => {
		const directory = newProject();
		const file = join(directory, "DataForge/pack.yml");
		const text = readFileSync(file, "utf8").replace("    entry: scripts/main.ts", "    entry: scripts/main.ts\n    capabilities: [kdf.items.define]");
		writeFileSync(file, text);
		assert.ok(codes(directory).includes("script.capabilities"));
	});
	it("rejects deterministic meta capabilities outside realm any", () => {
		const directory = newProject();
		const manifest = join(directory, "DataForge/pack.yml");
		writeFileSync(manifest, readFileSync(manifest, "utf8").replace("    entry: scripts/main.ts", "    entry: scripts/main.ts\n    realm: server"));
		const entry = join(directory, "DataForge/scripts/main.ts");
		writeFileSync(entry, readFileSync(entry, "utf8")
			.replace("import { defineHandler }", "import { defineHandler, defineMeta }")
			.replace("export default", "export const meta = defineMeta({ capabilities: ['kdf.reflection.write'] });\n\nexport default"));
		assert.ok(codes(directory).includes("script.deterministic-lane"));
	});
	it("rejects meta without the built-in defineMeta import", () => {
		const directory = newProject();
		const entry = join(directory, "DataForge/scripts/main.ts");
		writeFileSync(entry, readFileSync(entry, "utf8")
			.replace("export default", "const defineMeta = <T>(value: T): T => value;\nexport const meta = defineMeta({ capabilities: [] });\n\nexport default"));
		assert.ok(codes(directory).includes("entry.meta"));
	});
});

describe("import digests", () => {
	function withImport(): string {
		const directory = newProject();
		write(directory, "DataForge/data.txt", "hello");
		const file = join(directory, "DataForge/pack.yml");
		const text = readFileSync(file, "utf8").replace(
			"    entry: scripts/main.ts",
			`    entry: scripts/main.ts\n    imports:\n      - key: data\n        kind: text\n        path: data.txt\n        sha256: ${"0".repeat(64)}`,
		);
		writeFileSync(file, text);
		return directory;
	}
	it("detects a stale digest", () => {
		assert.ok(codes(withImport()).includes("import.digest-mismatch"));
	});
	it("kdf hash repairs it and keeps comments", async () => {
		const directory = withImport();
		const messages: string[] = [];
		const io = { out: (l: string) => messages.push(l), err: (l: string) => messages.push(l) };
		assert.equal(await run(["hash", directory], io, "0.0.0"), 0);
		assert.deepEqual(codes(directory), []);
		const pack = readFileSync(join(directory, "DataForge/pack.yml"), "utf8");
		assert.ok(pack.includes(sha256Of(join(directory, "DataForge/data.txt"))));
		assert.match(pack, /sha256:/);
	});
});

describe("manifest sync", () => {
	it("discovers imports and reflected YAML targets in stable order", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/main.ts", `import { defineHandler } from "@sf-kmods/kdataforgejs";
export default defineHandler(async (ctx) => {
	ctx.import("yaml", "patch.cdo.yml");
	ctx.import("text", "notes.txt");
});
`);
		write(directory, "DataForge/notes.txt", "hello\n");
		write(directory, "DataForge/patch.cdo.yml", `type: cdo
patches:
  - target: /Game/Z/BP_Z.BP_Z_C
    properties: []
  - target: /Game/A/BP_A.BP_A_C
    properties: []
`);

		const first = syncProject(directory);
		const firstManifest = readFileSync(join(directory, "DataForge/pack.yml"), "utf8");
		const second = syncProject(directory);

		assert.equal(first.changedPacks.length, 1);
		assert.deepEqual(second.changedPacks, []);
		assert.equal(readFileSync(join(directory, "DataForge/pack.yml"), "utf8"), firstManifest);
		assert.ok(firstManifest.indexOf("/Game/A/BP_A.BP_A_C") < firstManifest.indexOf("/Game/Z/BP_Z.BP_Z_C"));
		assert.ok(firstManifest.indexOf("path: notes.txt") < firstManifest.indexOf("path: patch.cdo.yml"));
		assert.match(firstManifest, new RegExp(sha256Of(join(directory, "DataForge/notes.txt"))));
		assert.match(firstManifest, new RegExp(sha256Of(join(directory, "DataForge/patch.cdo.yml"))));
		assert.deepEqual(codes(directory), []);
	});

	it("rejects dynamic import declarations that cannot be secured automatically", () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/main.ts", `import { defineHandler } from "@sf-kmods/kdataforgejs";
const file = "notes.txt";
export default defineHandler(async (ctx) => { ctx.import("text", file); });
`);
		assert.throws(() => syncProject(directory), /literal kind and path/);
	});
});

describe("check", () => {
	function checkable(): string {
		const directory = newProject();
		writeFileSync(join(directory, "tsconfig.json"), JSON.stringify({ extends: TYPES_BASE, include: ["DataForge/**/*.ts"] }));
		return directory;
	}
	it("passes for the generated project", () => {
		assert.deepEqual(checkProject(checkable()).map((d) => `${d.file} ${d.message}`), []);
	});
	it("resolves a relative project path before loading an extended config", () => {
		const directory = checkable();
		assert.deepEqual(checkProject(relative(process.cwd(), directory)).map((d) => `${d.file} ${d.message}`), []);
	});
	it("reports type errors", () => {
		const directory = checkable();
		write(directory, "DataForge/scripts/lib.ts", "export const n: number = 'text';\n");
		assert.ok(checkProject(directory).some((d) => d.code === "TS2322"));
	});
	it("prefers imported game-data configuration", () => {
		const directory = checkable();
		write(directory, "tsconfig.kdf.json", JSON.stringify({ compilerOptions: { noEmit: true }, files: ["DataForge/scripts/broken.ts"] }));
		write(directory, "DataForge/scripts/broken.ts", "const value: number = 'text';\n");
		assert.ok(checkProject(directory).some((d) => d.code === "TS2322"));
	});
});

describe("run", () => {
	const io = { out: () => undefined, err: () => undefined };
	it("formats pack scripts with bundled Biome", async () => {
		const directory = newProject();
		const sourcePath = "DataForge/scripts/nested/example.ts";
		write(directory, sourcePath, "export const example={enabled:true};\n");
		const lines: string[] = [];
		const exitCode = await run(["format", directory], {
			out: (line) => lines.push(line),
			err: (line) => lines.push(line),
		}, "0");

		assert.equal(exitCode, 0);
		assert.equal(readFileSync(join(directory, sourcePath), "utf8"),
			"export const example = { enabled: true };\n");
		assert.match(lines.join("\n"), /DataForge\/scripts\/nested\/example\.ts/);
	});
	it("reports Biome lint errors from pack scripts", async () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/lint-error.ts", "if (true) {}\n");
		const lines: string[] = [];
		const exitCode = await run(["lint", directory], {
			out: (line) => lines.push(line),
			err: (line) => lines.push(line),
		}, "0");

		assert.equal(exitCode, 1);
		assert.match(lines.join("\n"), /lint\/correctness\/noConstantCondition/);
	});
	it("accepts generated project without assist-only findings", async () => {
		const directory = newProject();
		write(directory, "DataForge/scripts/assist-only.ts", `import { defineMeta, defineHandler } from "@sf-kmods/kdataforgejs";
export const meta = defineMeta({ capabilities: [] });
export default defineHandler(() => {});
`);
		const lines: string[] = [];
		const exitCode = await run(["lint", directory], {
			out: (line) => lines.push(line),
			err: (line) => lines.push(line),
		}, "0");

		assert.equal(exitCode, 0);
		assert.deepEqual(lines, ["Lint passed."]);
	});
	it("prints version", async () => {
		const lines: string[] = [];
		assert.equal(await run(["--version"], { out: (l) => lines.push(l), err: () => undefined }, "1.2.3"), 0);
		assert.deepEqual(lines, ["1.2.3"]);
	});
	it("fails on unknown commands", async () => {
		assert.equal(await run(["nope"], io, "0"), 2);
	});
	it("dispatches game-data with installation, timeout, schema and cache options", async () => {
		const directory = newProject();
		const lines: string[] = [];
		let received: unknown;
		const runGameData = createCli({
			importGameData: async (options) => {
				received = options;
				return {
					installation: { source: "steam", channel: "stable", root: "C:/Satisfactory", executable: "C:/Satisfactory/FactoryGameSteam.exe" },
					archivePath: "Project/.kdataforge/satisfactory.kdfdata",
					declarationPath: "Project/DataForge/typescript/kdataforgejs/index.d.ts",
				};
			},
		});
		const exitCode = await runGameData(
			["game-data", directory, "--game-path", "C:/Satisfactory", "--timeout", "30", "--content-types", "--force"],
			{ out: (line) => lines.push(line), err: (line) => lines.push(line) },
			"0",
		);
		assert.equal(exitCode, 0);
		assert.equal((received as { destination: string }).destination, directory);
		assert.deepEqual((received as { projectRoots: string[] }).projectRoots, [directory]);
		assert.ok((received as { synchronizedFiles: unknown[] }).synchronizedFiles.length > 0);
		assert.equal((received as { gamePath: string }).gamePath, "C:/Satisfactory");
		assert.equal((received as { timeoutMilliseconds: number }).timeoutMilliseconds, 30_000);
		assert.equal((received as { contentTypes: boolean }).contentTypes, true);
		assert.equal((received as { force: boolean }).force, true);
		assert.match(lines.join("\n"), /Imported game data/);
	});
});
