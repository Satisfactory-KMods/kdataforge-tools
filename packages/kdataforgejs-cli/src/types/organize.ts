import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, sep } from "node:path";
import ts from "typescript";
import { formatGeneratedContent } from "./format.js";
import { strictContentDeclarations } from "./contentTypes.js";

const RUNTIME_MODULE = "@sf-kmods/kdataforgejs";
const SATISFACTORY_MODULE = "satisfactory";
const GENERATED_FILE = "kdataforge.generated.d.ts";

/** Splits exported ambient declarations into deterministic, navigable files and returns the entry declaration. */
export function organizeGeneratedDeclarations(typesDirectory: string, options: { readonly contentTypes?: boolean } = {}): string {
	const generatedPath = join(typesDirectory, GENERATED_FILE);
	const original = readFileSync(generatedPath, "utf8");
	const source = ts.createSourceFile(
		generatedPath,
		options.contentTypes ? strictContentDeclarations(original) : original,
		ts.ScriptTarget.ES2020,
		true,
		ts.ScriptKind.TS,
	);
	removePreviousOrganization(typesDirectory);
	const references: string[] = [];
	const runtimeDeclarations: string[] = [];
	const globalStatements: ts.Statement[] = [];

	for (const statement of source.statements) {
		if (!ts.isModuleDeclaration(statement)) {
			globalStatements.push(statement);
			continue;
		}
		const moduleName = stringModuleName(statement);
		if (moduleName === SATISFACTORY_MODULE) {
			references.push(...writeSatisfactoryGroups(typesDirectory, statement, source));
		} else if (moduleName === RUNTIME_MODULE && containsNativeClassPathMap(statement)) {
			references.push(...writeClassPathGroups(typesDirectory, statement, source));
			const remainingStatements = moduleStatements(statement)
				.filter((node) => !isNativeClassPathMap(node))
				.map((node) => node.getText(source));
			if (remainingStatements.length > 0) {
				runtimeDeclarations.push(ambientModule(RUNTIME_MODULE, remainingStatements));
			}
		} else {
			runtimeDeclarations.push(statement.getText(source));
		}
	}

	if (runtimeDeclarations.length > 0) {
		writeDeclaration(typesDirectory, "runtime.d.ts", runtimeDeclarations.join("\n\n"));
		references.push("runtime.d.ts");
	}
	if (globalStatements.length > 0) {
		writeDeclaration(typesDirectory, "globals.d.ts", globalStatements.map((node) => node.getText(source)).join("\n"));
		references.push("globals.d.ts");
	}

	references.sort();
	const moduleAnchors = `declare module "${RUNTIME_MODULE}" {}\ndeclare module "${SATISFACTORY_MODULE}" {}`;
	writeDeclaration(typesDirectory, "index.d.ts",
		`${references.map((path) => `/// <reference path="./${path}" />`).join("\n")}\n\n${moduleAnchors}`);
	updateGeneratedConfigs(typesDirectory);
	rmSync(generatedPath);
	return join(typesDirectory, "index.d.ts");
}

function removePreviousOrganization(typesDirectory: string): void {
	for (const path of ["globals.d.ts", "index.d.ts", "runtime.d.ts", "paths", "satisfactory"]) {
		rmSync(join(typesDirectory, path), { force: true, recursive: true });
	}
}

function writeSatisfactoryGroups(
	typesDirectory: string,
	declaration: ts.ModuleDeclaration,
	source: ts.SourceFile,
): string[] {
	const statements = moduleStatements(declaration);
	const imports = statements.filter(ts.isImportDeclaration).map((node) => node.getText(source));
	const groups = new Map<string, string[]>();
	for (const statement of statements) {
		if (ts.isImportDeclaration(statement)) continue;
		const key = satisfactoryGroup(statementName(statement));
		const entries = groups.get(key) ?? [];
		entries.push(statement.getText(source));
		groups.set(key, entries);
	}
	return [...groups.entries()].sort(([left], [right]) => left.localeCompare(right)).map(([key, entries]) => {
		const path = `satisfactory/${key}.d.ts`;
		writeDeclaration(typesDirectory, path, ambientModule(SATISFACTORY_MODULE, [...imports, ...entries]));
		return path;
	});
}

function writeClassPathGroups(
	typesDirectory: string,
	declaration: ts.ModuleDeclaration,
	source: ts.SourceFile,
): string[] {
	const pathMap = moduleStatements(declaration).find((statement): statement is ts.InterfaceDeclaration =>
		ts.isInterfaceDeclaration(statement) && statement.name.text === "NativeClassPathMap");
	if (pathMap === undefined) return [];
	const groups = new Map<string, ts.TypeElement[]>();
	for (const member of pathMap.members) {
		const key = classPathGroup(propertyPath(member));
		const entries = groups.get(key) ?? [];
		entries.push(member);
		groups.set(key, entries);
	}
	return [...groups.entries()].sort(([left], [right]) => left.localeCompare(right)).map(([key, members]) => {
		const body = members.map((member) => qualifiedTypeReference(member.getText(source))).join("\n");
		const path = `paths/${key}.d.ts`;
		writeDeclaration(typesDirectory, path,
			`import type * as Satisfactory from "satisfactory";\n\ndeclare module "${RUNTIME_MODULE}" {\n  interface NativeClassPathMap {\n${indent(body, 4)}\n  }\n}`);
		return path;
	});
}

function updateGeneratedConfigs(typesDirectory: string): void {
	updateJson(join(typesDirectory, "tsconfig.json"), (value) => {
		const compilerOptions = recordOrEmpty(value.compilerOptions);
		value.compilerOptions = compilerOptions;
		compilerOptions.paths = { [RUNTIME_MODULE]: ["index.d.ts"], [SATISFACTORY_MODULE]: ["index.d.ts"] };
	});
	updateJsonIfPresent(join(typesDirectory, "paths.json"), (value) => {
		value.paths = { [RUNTIME_MODULE]: "index.d.ts", [SATISFACTORY_MODULE]: "index.d.ts" };
	});
	updateJsonIfPresent(join(typesDirectory, "editor.json"), (value) => {
		const typescript = recordOrEmpty(value.typescript);
		value.typescript = typescript;
		typescript.declaration = "index.d.ts";
	});
}

function updateJsonIfPresent(path: string, update: (value: Record<string, unknown>) => void): void {
	if (existsSync(path)) updateJson(path, update);
}

function updateJson(path: string, update: (value: Record<string, unknown>) => void): void {
	const value = record(JSON.parse(readFileSync(path, "utf8")));
	update(value);
	writeFileSync(path, formatGeneratedContent(JSON.stringify(value), path));
}

function record(value: unknown): Record<string, unknown> {
	if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("Generated config must be an object.");
	return value as Record<string, unknown>;
}

function recordOrEmpty(value: unknown): Record<string, unknown> {
	return value === undefined ? {} : record(value);
}

function moduleStatements(declaration: ts.ModuleDeclaration): readonly ts.Statement[] {
	return declaration.body !== undefined && ts.isModuleBlock(declaration.body) ? declaration.body.statements : [];
}

function stringModuleName(declaration: ts.ModuleDeclaration): string | undefined {
	return ts.isStringLiteral(declaration.name) ? declaration.name.text : undefined;
}

function containsNativeClassPathMap(declaration: ts.ModuleDeclaration): boolean {
	return moduleStatements(declaration).some(isNativeClassPathMap);
}

function isNativeClassPathMap(statement: ts.Statement): boolean {
	return ts.isInterfaceDeclaration(statement) && statement.name.text === "NativeClassPathMap";
}

function statementName(statement: ts.Statement): string {
	if (ts.isInterfaceDeclaration(statement) || ts.isTypeAliasDeclaration(statement) ||
		ts.isClassDeclaration(statement) || ts.isFunctionDeclaration(statement)) return statement.name?.text ?? "misc";
	if (ts.isVariableStatement(statement)) return statement.declarationList.declarations[0]?.name.getText() ?? "misc";
	return "misc";
}

function satisfactoryGroup(name: string): string {
	if (name.startsWith("KdfEnumBrand")) return "e";
	const first = name[0]?.toLocaleLowerCase();
	return first !== undefined && /^[a-z0-9]$/u.test(first) ? first : "misc";
}

function propertyPath(member: ts.TypeElement): string {
	if (!ts.isPropertySignature(member) || member.name === undefined) return "misc";
	return ts.isStringLiteral(member.name) ? member.name.text : member.name.getText();
}

function classPathGroup(path: string): string {
	const segments = path.split("/").filter(Boolean);
	if (segments[0]?.toLocaleLowerCase() === "script") {
		return slug(["script", segments[1]?.split(".")[0] ?? "misc"]);
	}
	const count = Math.max(1, Math.min(3, segments.length - 1));
	return slug(segments.slice(0, count));
}

function slug(parts: readonly string[]): string {
	const value = parts.join("-").toLocaleLowerCase().replace(/[^a-z0-9]+/gu, "-").replace(/^-|-$/gu, "");
	return value === "" ? "misc" : value;
}

function qualifiedTypeReference(text: string): string {
	return text.replace(/import\(["']satisfactory["']\)\.([A-Za-z_$][\w$]*)/gu, "Satisfactory.$1");
}

function ambientModule(name: string, statements: readonly string[]): string {
	return `declare module "${name}" {\n${indent(statements.join("\n"), 2)}\n}`;
}

function indent(value: string, spaces: number): string {
	const prefix = " ".repeat(spaces);
	return value.split("\n").map((line) => line.length === 0 ? line : `${prefix}${line}`).join("\n");
}

function writeDeclaration(root: string, path: string, body: string): void {
	const target = join(root, path.split("/").join(sep));
	mkdirSync(dirname(target), { recursive: true });
	const content = `// Generated by KDataForgeJS. Do not edit.\n${body.trim()}\n`;
	writeFileSync(target, formatGeneratedContent(content, target));
}
