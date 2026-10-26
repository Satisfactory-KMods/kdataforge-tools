import { existsSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { spanInBytesToSpanInCodeUnits, type Diagnostic as BiomeDiagnostic } from "@biomejs/js-api/nodejs";
import type { Diagnostic } from "../diagnostics.js";
import { discoverPacks } from "../pack/discover.js";
import { findScriptFiles } from "../pack/scriptFiles.js";
import { biome, biomeProjectKey } from "../types/biome.js";

/** Lints every JavaScript and TypeScript source file in DataForge/scripts with bundled Biome rules. */
export function lintProject(projectRoot: string, packRef?: string): Diagnostic[] {
	const absoluteProjectRoot = resolve(projectRoot);
	const roots = discoverPacks(absoluteProjectRoot, packRef).packs.map((pack) => join(pack.root, "scripts"));
	const diagnostics: Diagnostic[] = roots.filter((scriptsRoot) => !existsSync(scriptsRoot)).map((scriptsRoot) => ({
			severity: "error",
			code: "lint.no-scripts",
			file: relative(absoluteProjectRoot, scriptsRoot),
			message: "Scripts directory not found. Run 'kdf init' or pass project root.",
		}));
	const files = new Set(roots.filter(existsSync).flatMap(findScriptFiles));
	return [...diagnostics, ...[...files].sort().flatMap((filePath) => lintFile(filePath, absoluteProjectRoot))];
}

function lintFile(filePath: string, projectRoot: string): Diagnostic[] {
	const content = readFileSync(filePath, "utf8");
	return biome.lintContent(biomeProjectKey, content, { filePath }).diagnostics
		.filter((diagnostic) => !diagnostic.category?.startsWith("assist/"))
		.map((diagnostic) => toDiagnostic(diagnostic, content, filePath, projectRoot));
}

function toDiagnostic(
	diagnostic: BiomeDiagnostic,
	content: string,
	filePath: string,
	projectRoot: string,
): Diagnostic {
	const location = diagnostic.location.span === undefined ? "" : diagnosticLocation(diagnostic.location.span, content);
	const shownPath = relative(projectRoot, filePath).split(sep).join("/");
	return {
		severity: diagnostic.severity === "error" || diagnostic.severity === "fatal" ? "error" : "warning",
		code: diagnostic.category ?? "biome",
		file: `${shownPath}${location}`,
		message: diagnostic.description,
	};
}

function diagnosticLocation(span: [number, number], content: string): string {
	const [start] = spanInBytesToSpanInCodeUnits(span, content);
	const before = content.slice(0, start);
	const line = before.split("\n").length;
	const lastNewline = before.lastIndexOf("\n");
	const column = start - lastNewline;
	return `:${line}:${column}`;
}
