import { existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import ts from "typescript";
import type { Diagnostic } from "../diagnostics.js";
import { discoverPacks } from "../pack/discover.js";

/** Type-checks the project's tsconfig.json (no emit) with the TypeScript compiler bundled with the CLI. */
export function checkProject(projectRoot: string, configName?: string, packRef?: string): Diagnostic[] {
	const selection = discoverPacks(projectRoot, packRef);
	const projects = [...new Set(selection.packs.map((pack) => pack.projectRoot))];
	return projects.flatMap((root) => checkSingleProject(root, selection.root, configName));
}

function checkSingleProject(projectRoot: string, displayRoot: string, configName?: string): Diagnostic[] {
	const absoluteProjectRoot = resolve(projectRoot);
	const selectedConfig = configName ?? (existsSync(join(absoluteProjectRoot, "tsconfig.kdf.json")) ? "tsconfig.kdf.json" : "tsconfig.json");
	const configPath = join(absoluteProjectRoot, selectedConfig);
	if (!existsSync(configPath)) {
		return [
			{
				severity: "error",
				code: "check.no-tsconfig",
				file: relative(displayRoot, configPath).split("\\").join("/"),
				message: "tsconfig.json not found. Run 'kdf init' or extend @sf-kmods/kdataforgejs/tsconfig.base.json.",
			},
		];
	}
	const read = ts.readConfigFile(configPath, ts.sys.readFile);
	if (read.error !== undefined) return [toDiagnostic(read.error, displayRoot)];
	const parsed = ts.parseJsonConfigFileContent(read.config, ts.sys, dirname(configPath), undefined, configPath);
	const program = ts.createProgram({ rootNames: parsed.fileNames, options: { ...parsed.options, noEmit: true } });
	return [...parsed.errors, ...ts.getPreEmitDiagnostics(program)].map((d) => toDiagnostic(d, displayRoot));
}

function toDiagnostic(diagnostic: ts.Diagnostic, projectRoot: string): Diagnostic {
	let file = "tsconfig.json";
	if (diagnostic.file !== undefined && diagnostic.start !== undefined) {
		const { line, character } = diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start);
		file = `${relative(projectRoot, diagnostic.file.fileName).split("\\").join("/")}:${line + 1}:${character + 1}`;
	}
	return {
		severity: diagnostic.category === ts.DiagnosticCategory.Error ? "error" : "warning",
		code: `TS${diagnostic.code}`,
		file,
		message: ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n").trim(),
	};
}
