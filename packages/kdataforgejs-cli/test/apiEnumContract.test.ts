import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { dirname } from "node:path";
import { createRequire } from "node:module";
import { it } from "node:test";
import ts from "typescript";
import { capabilityLiteral } from "../src/pack/capabilityEnums.js";
import { discoverPackUsage } from "../src/pack/usage.js";
import { syncConfigTypes } from "../src/types/configTypes.js";

it("resolves direct aliased Capability values and rejects lookalikes, type imports and computed access", () => {
	for (const [imports, expression, expected] of [
		['import { Capability as Cap } from "@sf-kmods/kdataforgejs";', "Cap.ImportsRead", "kdf.imports.read"],
		['import type { Capability as Cap } from "@sf-kmods/kdataforgejs";', "Cap.ImportsRead", undefined],
		['import { type Capability as Cap } from "@sf-kmods/kdataforgejs";', "Cap.ImportsRead", undefined],
		['const Cap = { ImportsRead: "kdf.imports.read" };', "Cap.ImportsRead", undefined],
		['import { Capability as Cap } from "@sf-kmods/kdataforgejs";', 'Cap["ImportsRead"]', undefined],
		['import { Capability as Cap } from "@sf-kmods/kdataforgejs";', "Cap.Missing", undefined],
	] as const) {
		const source = ts.createSourceFile("main.ts", `${imports}\n${expression};`, ts.ScriptTarget.Latest, true);
		const statement = source.statements[1];
		assert.ok(statement && ts.isExpressionStatement(statement));
		assert.equal(capabilityLiteral(statement.expression, source), expected);
	}
});

it("secures ImportKind members in the real source graph and refuses local shadows", () => {
	const root = mkdtempSync(join(tmpdir(), "kdf-import-enum-"));
	try {
		mkdirSync(join(root, "scripts"));
		writeFileSync(join(root, "notes.txt"), "notes");
		const path = join(root, "scripts/main.ts");
		const imports = 'import { ImportKind as Kind, ctx } from "@sf-kmods/kdataforgejs";';
		writeFileSync(path, `${imports}\nctx.import(Kind.Text, "notes.txt");`);
		assert.deepEqual(discoverPackUsage(root, "scripts/main.ts").imports, [{ kind: "text", path: "notes.txt" }]);
		writeFileSync(path, `${imports}\nfunction run(Kind: {Text:string}) { ctx.import(Kind.Text, "notes.txt"); }`);
		assert.throws(() => discoverPackUsage(root, "scripts/main.ts"), /ImportKind/);
	} finally { rmSync(root, { recursive: true, force: true }); }
});

it("generates executable selections, preserves scalar spelling and refuses authored output", async () => {
	const root = mkdtempSync(join(tmpdir(), "kdf-config-enums-"));
	try {
		writeFileSync(join(root, "select.config.yml"), `fields:
  - { id: quality, type: select, options: [{value: fast, label: Fast}, {value: careful, label: Careful}] }
  - { id: lexeme, type: select, options: [{value: 2.0, label: Two}] }
  - { id: exact, type: select, valueType: int64, options: [{value: 9007199254740993, label: Exact}] }
  - { id: switch, type: select, valueType: bool, options: [{value: yes, label: Yes}, {value: no, label: No}] }
`);
		const declaration = syncConfigTypes(root);
		const modulePath = join(root, "scripts/generated/kdataforgejs-config-enums.ts");
		const source = readFileSync(modulePath, "utf8");
		const javascript = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.ESNext } }).outputText;
		const values: Record<string, Record<string, unknown>> = await import(`data:text/javascript;base64,${Buffer.from(javascript).toString("base64")}`);
		const choice = (prefix: string) => values[Object.keys(values).find((key) => key.startsWith(prefix)) ?? ""];
		assert.equal(choice("ConfigQuality_")?.Fast, "fast");
		assert.equal(choice("ConfigLexeme_")?.Value0, "2.0");
		assert.equal(choice("ConfigExact_")?.Value0, 9007199254740993n);
		assert.equal(choice("ConfigSwitch_")?.Value0, true);
		assert.equal(choice("ConfigSwitch_")?.Value1, false);
		assert.match(readFileSync(declaration, "utf8"), /import\("\.\.\/scripts\/generated\/kdataforgejs-config-enums"\)/);
		const quality = Object.keys(values).find((key) => key.startsWith("ConfigQuality_"));
		const entry = join(root, "scripts/main.ts");
		writeFileSync(entry, `import {ctx} from "@sf-kmods/kdataforgejs";\nimport {${quality} as Quality} from "./generated/kdataforgejs-config-enums";\nconst selected: Quality | null = ctx.config.get("quality");\nconst valid: typeof selected = Quality.Fast;\n// @ts-expect-error selections require their enum type\nconst invalid: typeof selected = "fast";`);
		const typesRoot = dirname(createRequire(import.meta.url).resolve("@sf-kmods/kdataforgejs/declarations.json"));
		const program = ts.createProgram([entry, declaration, join(typesRoot, "index.d.ts"), join(typesRoot, "lib/kdfjs-es2020.d.ts")], {
			strict: true, noLib: true, types: [], noEmit: true, target: ts.ScriptTarget.ES2020,
			module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler,
		});
		assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => ts.flattenDiagnosticMessageText(d.messageText, " ")), []);
		syncConfigTypes(root);
		assert.equal(readFileSync(modulePath, "utf8"), source);
		writeFileSync(modulePath, "export const authored = true;");
		assert.throws(() => syncConfigTypes(root), /Refusing to overwrite authored/);
	} finally { rmSync(root, { recursive: true, force: true }); }
});
