import assert from "node:assert/strict";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, it } from "node:test";
import { fileURLToPath } from "node:url";
import { addScaffold, type AddKind } from "../src/commands/add.js";
import { run } from "../src/cli.js";
import { checkProject } from "../src/commands/check.js";
import { startDev } from "../src/commands/dev.js";
import { doctorProject } from "../src/commands/doctor.js";
import { initProject } from "../src/commands/init.js";
import { lintProject } from "../src/commands/lint.js";
import { syncProject } from "../src/commands/sync.js";
import { validateProject } from "../src/commands/validate.js";

const roots: string[] = [];
const TYPES_BASE = resolve(dirname(fileURLToPath(import.meta.url)), "../../../kdataforgejs-types/tsconfig.base.json");
function project(): string {
	const root = mkdtempSync(join(tmpdir(), "kdf-authoring-"));
	roots.push(root);
	initProject({ directory: root, force: false });
	return root;
}
after(() => { for (const root of roots) rmSync(root, { recursive: true, force: true }); });

it("doctor reports manifest drift without changing pack.yml", () => {
	const root = project();
	const entry = join(root, "DataForge/scripts/main.ts");
	writeFileSync(join(root, "DataForge/note.txt"), "hello");
	writeFileSync(entry, readFileSync(entry, "utf8").replace("console.info", "ctx.import(\"text\", \"note.txt\");\n\tconsole.info"));
	const manifest = join(root, "DataForge/pack.yml");
	const before = readFileSync(manifest, "utf8");
	const checks = doctorProject(root);
	assert.equal(readFileSync(manifest, "utf8"), before);
	assert.equal(checks.find((check) => check.name === "manifest")?.status, "warn");
});

it("doctor --json emits structured findings without writing", async () => {
	const root = project();
	const manifest = join(root, "DataForge/pack.yml");
	const before = readFileSync(manifest, "utf8");
	const lines: string[] = [];
	await run(["doctor", root, "--json"], { out: (line) => lines.push(line), err: (line) => lines.push(line) }, "0");
	const report = JSON.parse(lines.join("\n")) as { checks: Array<{ name: string; status: string }> };
	assert.equal(report.checks.find((item) => item.name === "manifest")?.status, "warn");
	assert.equal(readFileSync(manifest, "utf8"), before);
	assert.equal(existsSync(join(root, "DataForge/typescript/kdataforgejs-config.d.ts")), false);
});

it("creates each scaffold once and keeps its manifest valid", () => {
	for (const kind of ["handler", "hook", "item", "recipe", "document"] as AddKind[]) {
		const root = project();
		const path = addScaffold(root, kind, "example");
		assert.ok(existsSync(path));
		assert.throws(() => addScaffold(root, kind, "example"), /Refusing to overwrite/);
		syncProject(root);
		assert.deepEqual(validateProject(root).map((item) => item.code), []);
		writeFileSync(join(root, "tsconfig.json"), JSON.stringify({ extends: TYPES_BASE, include: ["DataForge/**/*.ts"] }));
		assert.deepEqual(checkProject(root).map((item) => `${item.code}: ${item.message}`), []);
		assert.deepEqual(lintProject(root).map((item) => `${item.code}: ${item.message}`), []);
	}
});

it("rejects computed metadata before creating a file", () => {
	const root = project();
	const entry = join(root, "DataForge/scripts/main.ts");
	const before = readFileSync(entry, "utf8")
		.replace("import { defineHandler }", "import { defineHandler, defineMeta }")
		.replace("export default", "export const meta = defineMeta({ capabilities: requested });\nexport default");
	writeFileSync(entry, before);
	assert.throws(() => addScaffold(root, "item", "newItem"), /Computed or nonliteral/);
	assert.equal(readFileSync(entry, "utf8"), before);
	assert.equal(existsSync(join(root, "DataForge/scripts/newItem.ts")), false);
});

it("normalizes mixed-case scaffold names into valid function names", () => {
	const root = project();
	addScaffold(root, "handler", "My-Handler");
	assert.match(readFileSync(join(root, "DataForge/scripts/main.ts"), "utf8"), /addMyHandler\(ctx\)/u);
	assert.deepEqual(validateProject(root).map((item) => item.code), []);
});

it("adds two documents with one HandlerDocument import", () => {
	const root = project();
	addScaffold(root, "document", "first");
	addScaffold(root, "document", "second", "recipe");
	const entry = readFileSync(join(root, "DataForge/scripts/main.ts"), "utf8");
	assert.equal(entry.match(/import type \{ HandlerDocument \}/gu)?.length, 1);
	assert.match(entry, /first\.yml/u);
	assert.match(entry, /second\.yml/u);
	syncProject(root);
	assert.deepEqual(validateProject(root).map((item) => item.code), []);
	writeFileSync(join(root, "tsconfig.json"), JSON.stringify({ extends: TYPES_BASE, include: ["DataForge/**/*.ts"] }));
	assert.deepEqual(checkProject(root).map((item) => item.code), []);
});

it("adds capability after a trailing comma without making an array hole", () => {
	const root = project();
	const path = join(root, "DataForge/scripts/main.ts");
	writeFileSync(path, readFileSync(path, "utf8")
		.replace("import { defineHandler }", "import { defineHandler, defineMeta }")
		.replace("export default", "export const meta = defineMeta({ capabilities: [\"kdf.imports.read\",] });\nexport default"));
	addScaffold(root, "item", "ExampleItem");
	const entry = readFileSync(path, "utf8");
	assert.match(entry, /capabilities: \["kdf\.imports\.read", "kdf\.items\.define"\]/u);
	assert.deepEqual(validateProject(root).map((item) => item.code), []);
});

it("dev reruns after authored change and ignores generated manifest writes", async () => {
	const root = project();
	writeFileSync(join(root, "tsconfig.json"), JSON.stringify({ extends: TYPES_BASE, include: ["DataForge/**/*.ts"] }));
	const messages: string[] = [];
	const watcher = startDev(root, { out: (line) => messages.push(line), err: (line) => messages.push(line) });
	try {
		await until(() => messages.some((line) => line.includes("Checks passed")));
		const initial = messages.filter((line) => line.includes("Checks passed")).length;
		const lib = join(root, "DataForge/scripts/lib.ts");
		writeFileSync(lib, `${readFileSync(lib, "utf8")}\n// updated\n`);
		await until(() => messages.filter((line) => line.includes("Checks passed")).length > initial);
		await new Promise((done) => setTimeout(done, 400));
		assert.equal(messages.filter((line) => line.includes("Checks passed")).length, initial + 1);
	} finally { watcher.close(); }
});

it("dev reattaches watchers after DataForge is removed and recreated", async () => {
	const root = project();
	writeFileSync(join(root, "tsconfig.json"), JSON.stringify({ extends: TYPES_BASE, include: ["DataForge/**/*.ts"] }));
	const backup = mkdtempSync(join(tmpdir(), "kdf-watch-backup-"));
	roots.push(backup);
	cpSync(join(root, "DataForge"), join(backup, "DataForge"), { recursive: true });
	const messages: string[] = [];
	const watcher = startDev(root, { out: (line) => messages.push(line), err: (line) => messages.push(line) });
	try {
		await until(() => messages.some((line) => line.includes("Checks passed")));
		const beforeRecreate = messages.filter((line) => line.includes("Checks passed")).length;
		rmSync(join(root, "DataForge"), { recursive: true, force: true });
		cpSync(join(backup, "DataForge"), join(root, "DataForge"), { recursive: true });
		await until(() => messages.filter((line) => line.includes("Checks passed")).length > beforeRecreate);
		const beforeEdit = messages.filter((line) => line.includes("Checks passed")).length;
		const lib = join(root, "DataForge/scripts/lib.ts");
		writeFileSync(lib, `${readFileSync(lib, "utf8")}\n// after recreate\n`);
		await until(() => messages.filter((line) => line.includes("Checks passed")).length > beforeEdit);
	} finally { watcher.close(); }
});

async function until(predicate: () => boolean): Promise<void> {
	const deadline = Date.now() + 4_000;
	while (!predicate()) {
		if (Date.now() > deadline) throw new Error("Timed out waiting for dev pipeline");
		await new Promise((done) => setTimeout(done, 25));
	}
}
