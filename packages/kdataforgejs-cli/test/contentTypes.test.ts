import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, describe, it } from "node:test";
import ts from "typescript";
import { strictContentDeclarations } from "../src/types/contentTypes.js";
import { createCli } from "../src/cli.js";
import { organizeGeneratedDeclarations } from "../src/types/organize.js";
import { initProject } from "../src/commands/init.js";

const roots: string[] = [];
after(() => { for (const root of roots) rmSync(root, { recursive: true, force: true }); });

function diagnostics(code: string, declaration?: string): readonly ts.Diagnostic[] {
	const root = mkdtempSync(join(tmpdir(), "kdf-content-types-"));
	roots.push(root);
	const file = join(root, "example.ts");
	const source = declaration ?? 'declare module "@sf-kmods/kdataforgejs" { export interface SchematicDocument { readonly type: "schematic"; readonly schematics: readonly Record<string, unknown>[]; } }';
	writeFileSync(join(root, "types.d.ts"), strictContentDeclarations(source));
	writeFileSync(file, `import type { SchematicDocument } from "@sf-kmods/kdataforgejs";\n${code}`);
	return ts.getPreEmitDiagnostics(ts.createProgram([file, join(root, "types.d.ts")], { target: ts.ScriptTarget.ES2020, strict: true, noEmit: true, skipLibCheck: false, types: [] }));
}

describe("optional schema-derived content types", () => {
	it("typechecks every content schema against the actual runtime declaration", () => {
		const source = readFileSync(new URL("../../../kdataforgejs-types/index.d.ts", import.meta.url), "utf8");
		assert.deepEqual(diagnostics("", source).map((item) => ts.flattenDiagnosticMessageText(item.messageText, " ")), []);
	});
	it("organizes strict types only when the option is enabled", () => {
		const source = 'declare module "@sf-kmods/kdataforgejs" { export interface SchematicDocument { readonly type: "schematic"; readonly schematics: readonly Record<string, unknown>[]; } }';
		for (const contentTypes of [false, true]) {
			const root = mkdtempSync(join(tmpdir(), "kdf-content-organize-"));
			roots.push(root);
			writeFileSync(join(root, "kdataforge.generated.d.ts"), source);
			writeFileSync(join(root, "tsconfig.json"), "{}");
			organizeGeneratedDeclarations(root, { contentTypes });
			const output = readFileSync(join(root, "runtime.d.ts"), "utf8");
			assert.equal(output.includes("type SchematicDocument"), contentTypes);
			assert.equal(output.includes("interface SchematicDocument"), !contentTypes);
		}
	});
	it("accepts a schematic with declared unlock fields", () => {
		assert.deepEqual(diagnostics('const doc: SchematicDocument = {type:"schematic", schematics:[{id:"Demo", inventorySlots:3}]};').map((item) => item.messageText), []);
	});
	it("rejects an unknown schematic property", () => {
		assert.ok(diagnostics('const doc: SchematicDocument = {type:"schematic", schematics:[{id:"Demo", inventroySlots:3}]};').some((item) => ts.flattenDiagnosticMessageText(item.messageText, " ").includes("inventroySlots")));
	});
	it("rejects a missing schematic identity", () => {
		assert.ok(diagnostics('const doc: SchematicDocument = {type:"schematic", schematics:[{inventorySlots:3}]};').length > 0);
	});
	it("rejects wrong property types", () => {
		assert.ok(diagnostics('const doc: SchematicDocument = {type:"schematic", schematics:[{id:"Demo", inventorySlots:"three"}]};').length > 0);
	});
	it("forwards --content-types only when requested", async () => {
		const captured: boolean[] = [];
		const cli = createCli({ importGameData: async (options) => {
			captured.push(options.contentTypes === true);
			return { installation: { root: "fixture", executable: "fixture", source: "steam", channel: "stable" }, archivePath: "archive", declarationPath: "types" };
		} });
		const io = { out: () => {}, err: () => {} };
		const root = mkdtempSync(join(tmpdir(), "kdf-content-option-"));
		roots.push(root);
		initProject({ directory: root, force: false });
		assert.equal(await cli(["game-data", root, "--content-types"], io, "test"), 0);
		assert.equal(await cli(["game-data", root], io, "test"), 0);
		assert.deepEqual(captured, [true, false]);
	});
});
