import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { it } from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const types = resolve(dirname(fileURLToPath(import.meta.url)), "../../../kdataforgejs-types");

function check(source: string): readonly string[] {
	const directory = mkdtempSync(join(tmpdir(), "kdf-enum-types-"));
	try {
		const file = join(directory, "main.ts");
		writeFileSync(file, source);
		writeFileSync(join(directory, "bridge.ts"), 'export { TypeKind as Kind } from "@sf-kmods/kdataforgejs";');
		const program = ts.createProgram([file, join(types, "index.d.ts"), join(types, "lib/kdfjs-es2020.d.ts")], {
			strict: true, noEmit: true, noLib: true, types: [], target: ts.ScriptTarget.ES2020,
			module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler,
		});
		return ts.getPreEmitDiagnostics(program).map(diagnostic =>
			ts.flattenDiagnosticMessageText(diagnostic.messageText, " "));
	} finally { rmSync(directory, { recursive: true, force: true }); }
}

it("exports real enum values through named, aliased, namespace and reexports", () => {
	assert.deepEqual(check(`import { TypeKind, TypeKind as Kind, MemberAccess, InvocationKind, CallEffect,
		TargetKind, TargetCardinality, Authority, ThreadAffinity, LifecyclePhase, CompareOperator, ctx } from "@sf-kmods/kdataforgejs";
		import * as sdk from "@sf-kmods/kdataforgejs";
		import { Kind as Reexported } from "./bridge";
		const kind: TypeKind = TypeKind.Struct;
		const alias: TypeKind = Kind.Struct;
		const namespace: TypeKind = sdk.TypeKind.Struct;
		const reexported: TypeKind = Reexported.Struct;
		const named: keyof typeof TypeKind = "Struct";
		const numeric: 1 = TypeKind.Struct;
		const reverse: string = TypeKind[1];
		const previousValues: [TypeKind, MemberAccess, InvocationKind, CallEffect, TargetKind,
			TargetCardinality, Authority, ThreadAffinity, LifecyclePhase] = [4, 3, 3, 3, 4, 1, 2, 1, 2];
		const enumValues = [MemberAccess.ReadWrite, InvocationKind.Static, CallEffect.ReadOnly, TargetKind.CDO,
			TargetCardinality.Many, Authority.Both, ThreadAffinity.GameThreadOnly, LifecyclePhase.Runtime];
		const comparison: CompareOperator = CompareOperator.Equal;
		const compared: boolean = ctx.compare(1, CompareOperator.Less, 2);
		for (const operator of [CompareOperator.Equal, CompareOperator.Less, CompareOperator.GreaterThan]) ctx.compare(1, operator, 2);
		// @ts-expect-error raw operators migrate to CompareOperator members
		ctx.compare(1, "equals", 2);
	`), []);
});

it("rejects unknown enum members, invalid numeric values and unrelated enum types", () => {
	assert.deepEqual(check(`import { TypeKind, MemberAccess, CompareOperator, ctx } from "@sf-kmods/kdataforgejs";
		// @ts-expect-error unknown enum member
		const missing = TypeKind.Missing;
		// @ts-expect-error literal is outside the enum
		const invalid: TypeKind = 99;
		// @ts-expect-error unrelated enum with the same numeric value
		const wrongEnum: TypeKind = MemberAccess.Read;
		// @ts-expect-error CompareOperator is a real string enum
		const rawEnum: CompareOperator = "equals";
		// @ts-expect-error string enum has no numeric reverse mapping
		const reverse = CompareOperator[0];
		// @ts-expect-error invalid historic operator
		ctx.compare(1, "invalid", 2);
	`), []);
});
