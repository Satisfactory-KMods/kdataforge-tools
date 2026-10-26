import { existsSync, readFileSync, realpathSync } from "node:fs";
import { dirname, extname, relative, resolve, sep } from "node:path";
import ts from "typescript";
import { RUNTIME_MODULE, SATISFACTORY_MODULE } from "../conventions.js";
import type { Diagnostic } from "../diagnostics.js";
import { isInside } from "./discover.js";

/** Walks the static import graph from the entry and reports everything the runtime would reject. */
export function checkSourceGraph(packRoot: string, entry: string): Diagnostic[] {
	const diagnostics: Diagnostic[] = [];
	const visited = new Set<string>();
	const entryFile = resolve(packRoot, entry);
	const scriptsRoot = dirname(entryFile);
	const queue = [entryFile];

	while (queue.length > 0) {
		const file = queue.pop() as string;
		if (visited.has(file)) continue;
		visited.add(file);
		const shown = relative(packRoot, file).split(sep).join("/");
		if (!existsSync(file)) {
			diagnostics.push({ severity: "error", code: "source.missing", file: shown, message: "Module file does not exist." });
			continue;
		}
		const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ES2020, true);
		const report = (code: string, message: string): void => {
			diagnostics.push({ severity: "error", code, file: shown, message });
		};
		visit(source, (node) => {
			const specifier = staticSpecifier(node);
			if (specifier !== undefined) {
				const target = resolveSpecifier(scriptsRoot, file, specifier, report);
				if (target !== undefined) queue.push(target);
			} else if (isDynamicImport(node) || isRequireCall(node)) {
				report("source.dynamic-import", "Dynamic import() and require() are not supported; use static imports.");
			}
		});
	}
	return diagnostics;
}
function visit(node: ts.Node, callback: (node: ts.Node) => void): void {
	callback(node);
	ts.forEachChild(node, (child) => visit(child, callback));
}

function staticSpecifier(node: ts.Node): string | undefined {
	if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier !== undefined) {
		return ts.isStringLiteral(node.moduleSpecifier) ? node.moduleSpecifier.text : undefined;
	}
	return undefined;
}

function isDynamicImport(node: ts.Node): boolean {
	return ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword;
}

function isRequireCall(node: ts.Node): boolean {
	return ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "require";
}

/** Mirrors FKDFJSModuleLexicalScanner::ResolveStaticImport: extensionless means `.ts`, modules stay inside scripts/. */
function resolveSpecifier(
	scriptsRoot: string,
	importer: string,
	specifier: string,
	report: (code: string, message: string) => void,
): string | undefined {
	if (specifier === RUNTIME_MODULE || specifier === SATISFACTORY_MODULE) return undefined;
	if (!specifier.startsWith("./") && !specifier.startsWith("../")) {
		report(
			"source.external-import",
			`Import '${specifier}' must be '${RUNTIME_MODULE}', '${SATISFACTORY_MODULE}', or a relative module.`,
		);
		return undefined;
	}
	const extension = extname(specifier);
	if (extension !== "" && extension !== ".ts" && extension !== ".js") {
		report("source.import-extension", `Relative import '${specifier}' must be a .ts or .js module.`);
		return undefined;
	}
	const target = resolve(dirname(importer), extension === "" ? `${specifier}.ts` : specifier);
	if (!isInside(scriptsRoot, target) || (existsSync(target) && !isInside(realpathSync(scriptsRoot), realpathSync(target)))) {
		report("source.outside-scripts", `Import '${specifier}' leaves the scripts folder.`);
		return undefined;
	}
	return target;
}

/** Entry metadata is optional; default handler is required. */
export function checkEntryShape(packRoot: string, entry: string): Diagnostic[] {
	const file = resolve(packRoot, entry);
	if (!existsSync(file)) return [];
	const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ES2020, true);
	let hasDefault = false;
	for (const statement of source.statements) {
		if (ts.isExportAssignment(statement) && !statement.isExportEquals) hasDefault = true;
	}
	const diagnostics: Diagnostic[] = [];
	if (!hasDefault) diagnostics.push({ severity: "error", code: "entry.handler", file: entry, message: "Entry must default-export `defineHandler(async (ctx) => ...)`." });
	return diagnostics;
}

export interface EntryMeta {
	readonly capabilities: readonly string[];
	readonly diagnostics: readonly Diagnostic[];
}

/** Mirrors runtime's literal-only optional defineMeta contract without executing pack code. */
export function readEntryMeta(packRoot: string, entry: string): EntryMeta {
	const file = resolve(packRoot, entry);
	if (!existsSync(file)) return { capabilities: [], diagnostics: [] };
	const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ES2020, true);
	const diagnostics: Diagnostic[] = [];
	const hasDefineMetaImport = source.statements.some((statement) => ts.isImportDeclaration(statement) &&
		ts.isStringLiteral(statement.moduleSpecifier) && statement.moduleSpecifier.text === RUNTIME_MODULE &&
		statement.importClause?.namedBindings && ts.isNamedImports(statement.importClause.namedBindings) &&
		statement.importClause.namedBindings.elements.some((element) => !element.propertyName && element.name.text === "defineMeta"));
	const metaDeclarations = source.statements.flatMap((statement) =>
		ts.isVariableStatement(statement) && ts.getModifiers(statement)?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword)
			? statement.declarationList.declarations.filter((declaration) => ts.isIdentifier(declaration.name) && declaration.name.text === "meta")
			: []);
	if (metaDeclarations.length === 0) return { capabilities: [], diagnostics };
	if (!hasDefineMetaImport) diagnostics.push({ severity: "error", code: "entry.meta", file: entry,
		message: "Optional meta requires an unaliased defineMeta import from @sf-kmods/kdataforgejs." });
	const declaration = metaDeclarations.length === 1 ? metaDeclarations[0] : undefined;
	const call = declaration?.initializer;
	const argument = call && ts.isCallExpression(call) ? call.arguments.at(0) : undefined;
	const object = call && ts.isCallExpression(call) && ts.isIdentifier(call.expression) && call.expression.text === "defineMeta" &&
		call.arguments.length === 1 && argument && ts.isObjectLiteralExpression(argument) ? argument : undefined;
	const firstProperty = object?.properties.at(0);
	const property = object?.properties.length === 1 && firstProperty && ts.isPropertyAssignment(firstProperty) &&
		ts.isIdentifier(firstProperty.name) && firstProperty.name.text === "capabilities" ? firstProperty : undefined;
	const array = property && ts.isArrayLiteralExpression(property.initializer) ? property.initializer : undefined;
	if (!array || array.elements.some((element) => !ts.isStringLiteral(element))) {
		diagnostics.push({ severity: "error", code: "entry.meta", file: entry,
			message: "Optional meta must be `defineMeta({ capabilities: [literal ids] })`." });
		return { capabilities: [], diagnostics };
	}
	const capabilities = array.elements.map((element) => (element as ts.StringLiteral).text);
	if (new Set(capabilities).size !== capabilities.length) diagnostics.push({ severity: "error", code: "entry.meta", file: entry,
		message: "Meta capability ids must be unique." });
	return { capabilities, diagnostics };
}
