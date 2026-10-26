import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import ts from "typescript";
import { parseDocument } from "yaml";
import { RUNTIME_MODULE } from "../conventions.js";
import { assertSafeWrite } from "../pack/transaction.js";

export type AddKind = "handler" | "hook" | "item" | "recipe" | "document";
const CAPABILITY: Partial<Record<AddKind, string>> = {
	hook: "kdf.native.hook", item: "kdf.items.define", recipe: "kdf.recipes.define", document: "kdf.documents.apply",
};
const NAME = /^[A-Za-z][A-Za-z0-9_-]*$/u;

export function addScaffold(directory: string, kind: AddKind, name: string, documentType = "cdo", selectedPackRoot?: string): string {
	if (!NAME.test(name)) throw new Error("Name must start with a letter and contain only letters, digits, '-' or '_'.");
	if (!(["handler", "hook", "item", "recipe", "document"] as string[]).includes(kind)) throw new Error(`Unsupported scaffold '${kind}'.`);
	if (kind === "document" && !NAME.test(documentType)) throw new Error("Document type must be a safe identifier.");
	const root = resolve(directory);
	const packRoot = selectedPackRoot ?? join(root, "DataForge");
	const entryPath = join(packRoot, "scripts/main.ts");
	assertSafeWrite(packRoot, entryPath);
	if (!existsSync(entryPath)) throw new Error(`Entry file does not exist: ${entryPath}`);
	const destination = kind === "document" ? join(packRoot, `${name}.yml`) : join(packRoot, "scripts", `${name}.ts`);
	assertSafeWrite(packRoot, destination);
	if (existsSync(destination)) throw new Error(`Refusing to overwrite ${destination}`);
	const before = readFileSync(entryPath, "utf8");
	const functionName = `add${name.split(/[-_]/u).map((part) => `${part[0]?.toUpperCase() ?? ""}${part.slice(1)}`).join("")}`;
	const updated = editEntry(before, kind, name, functionName, documentType);
	const content = scaffold(kind, name, functionName, documentType);
	if (kind === "document") {
		const parsed = parseDocument(content);
		if (parsed.errors.length > 0) throw new Error(parsed.errors[0]?.message);
	}
	mkdirSync(dirname(destination), { recursive: true });
	writeFileSync(destination, content, { flag: "wx" });
	const stagedEntry = `${entryPath}.${randomUUID()}.tmp`;
	try {
		if (readFileSync(entryPath, "utf8") !== before) throw new Error("Entry changed while scaffolding; retry.");
		writeFileSync(stagedEntry, updated, { flag: "wx" });
		renameSync(stagedEntry, entryPath);
	} catch (error) {
		rmSync(stagedEntry, { force: true });
		rmSync(destination);
		throw error;
	}
	return destination;
}

function scaffold(kind: AddKind, name: string, functionName: string, documentType: string): string {
	if (kind === "document") {
		const field = ({ cdo: "patches", item: "items", recipe: "recipes", asset: "assets", dataasset: "assets" } as Record<string, string>)[documentType];
		if (!field) throw new Error(`Document type '${documentType}' has no minimal scaffold; create it manually.`);
		return `type: ${documentType}\n${field}: []\n`;
	}
	if (kind === "item") return `import type { Context } from "${RUNTIME_MODULE}";\n\nexport function ${functionName}(ctx: Context): void {\n\tctx.createItem({\n\t\tid: "${name}",\n\t\tparent: { classPath: "/Script/FactoryGame.FGItemDescriptor" },\n\t\tdisplayName: { namespace: "${name}", key: "name", source: "${name}" },\n\t\tform: "solid",\n\t\tstackSize: "small",\n\t});\n}\n`;
	if (kind === "recipe") return `import type { Context } from "${RUNTIME_MODULE}";\n\nexport function ${functionName}(ctx: Context): void {\n\tctx.createRecipe({\n\t\tid: "${name}",\n\t\tingredients: [],\n\t\tproducts: [],\n\t\tproducedIn: [],\n\t\tdurationSeconds: 1,\n\t});\n}\n`;
	if (kind === "hook") return `import type { Context } from "${RUNTIME_MODULE}";\n\nexport function ${functionName}(ctx: Context): void {\n\t// Add ctx.hook(target, hook, callback) after selecting a reflected native target.\n\tvoid ctx;\n}\n`;
	return `import type { Context } from "${RUNTIME_MODULE}";\n\nexport function ${functionName}(ctx: Context): void {\n\tconsole.info(ctx.catalog().digest);\n}\n`;
}

interface Edit { readonly start: number; readonly end: number; readonly text: string }

function editEntry(before: string, kind: AddKind, name: string, functionName: string, documentType: string): string {
	const source = ts.createSourceFile("main.ts", before, ts.ScriptTarget.ES2022, true);
	if (ts.transpileModule(before, { reportDiagnostics: true, compilerOptions: { target: ts.ScriptTarget.ES2022 } })
		.diagnostics?.some((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error)) {
		throw new Error("Entry has syntax errors; refusing to edit it.");
	}
	const edits: Edit[] = [];
	if (kind !== "document" && source.statements.some((node) =>
		(ts.isFunctionDeclaration(node) && node.name?.text === functionName) ||
		(ts.isVariableStatement(node) && node.declarationList.declarations.some((declaration) => ts.isIdentifier(declaration.name) && declaration.name.text === functionName)) ||
		(ts.isImportDeclaration(node) && node.importClause?.namedBindings && ts.isNamedImports(node.importClause.namedBindings) &&
			node.importClause.namedBindings.elements.some((element) => element.name.text === functionName)))) {
		throw new Error(`Entry already declares '${functionName}'; refusing ambiguous edit.`);
	}
	const capability = CAPABILITY[kind];
	if (capability) addCapability(source, capability, edits);
	const assignment = source.statements.find((node) => ts.isExportAssignment(node) && !node.isExportEquals);
	if (!assignment || !ts.isExportAssignment(assignment) || !ts.isCallExpression(assignment.expression) ||
		!ts.isIdentifier(assignment.expression.expression) || assignment.expression.expression.text !== "defineHandler") {
		throw new Error("Entry must default-export a direct defineHandler call; refusing ambiguous edit.");
	}
	const callback = assignment.expression.arguments[0];
	if (!callback || (!ts.isArrowFunction(callback) && !ts.isFunctionExpression(callback)) || !ts.isBlock(callback.body)) {
		throw new Error("defineHandler callback must have a block body; refusing ambiguous edit.");
	}
	const ctx = callback.parameters[0]?.name;
	if (!ctx || !ts.isIdentifier(ctx)) throw new Error("Cannot identify handler context parameter.");
	const invocation = kind === "document"
		? `${ctx.text}.applyDocument("${documentType}", ${ctx.text}.import("yaml", "${name}.yml") as HandlerDocument<"${documentType}">);`
		: `${functionName}(${ctx.text});`;
	edits.push({ start: callback.body.end - 1, end: callback.body.end - 1, text: `\n\t${invocation}\n` });
	if (kind === "document") {
		const hasHandlerDocument = source.statements.some((node) => ts.isImportDeclaration(node) &&
			ts.isStringLiteral(node.moduleSpecifier) && node.moduleSpecifier.text === RUNTIME_MODULE &&
			node.importClause?.namedBindings && ts.isNamedImports(node.importClause.namedBindings) &&
			node.importClause.namedBindings.elements.some((element) => element.name.text === "HandlerDocument" && !element.propertyName));
		if (!hasHandlerDocument) edits.push({ start: 0, end: 0, text: `import type { HandlerDocument } from "${RUNTIME_MODULE}";\n` });
	} else edits.push({ start: 0, end: 0, text: `import { ${functionName} } from "./${name}";\n` });
	return edits.sort((left, right) => right.start - left.start).reduce((content, edit) =>
		content.slice(0, edit.start) + edit.text + content.slice(edit.end), before);
}

function addCapability(source: ts.SourceFile, capability: string, edits: Edit[]): void {
	const imported = source.statements.some((node) => ts.isImportDeclaration(node) &&
		ts.isStringLiteral(node.moduleSpecifier) && node.moduleSpecifier.text === RUNTIME_MODULE &&
		!node.importClause?.isTypeOnly && node.importClause?.namedBindings && ts.isNamedImports(node.importClause.namedBindings) &&
		node.importClause.namedBindings.elements.some((element) => element.name.text === "defineMeta" && !element.propertyName && !element.isTypeOnly));
	const meta = source.statements.flatMap((node) => ts.isVariableStatement(node) &&
		node.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ExportKeyword)
		? node.declarationList.declarations.filter((declaration) => ts.isIdentifier(declaration.name) && declaration.name.text === "meta") : []);
	if (meta.length > 1) throw new Error("Multiple exported meta declarations; refusing ambiguous edit.");
	if (meta.length === 0) {
		if (!imported && source.statements.some((node) => ts.isVariableStatement(node) &&
			node.declarationList.declarations.some((declaration) => ts.isIdentifier(declaration.name) && declaration.name.text === "defineMeta"))) {
			throw new Error("Local defineMeta binding conflicts with required import; refusing ambiguous edit.");
		}
		if (!imported) edits.push({ start: 0, end: 0, text: `import { defineMeta } from "${RUNTIME_MODULE}";\n` });
		const assignment = source.statements.find((node) => ts.isExportAssignment(node));
		if (!assignment) throw new Error("No default export for metadata insertion.");
		edits.push({ start: assignment.getStart(source), end: assignment.getStart(source),
			text: `export const meta = defineMeta({ capabilities: ["${capability}"] });\n\n` });
		return;
	}
	if (!imported) throw new Error("Existing meta lacks an unaliased value defineMeta import; refusing ambiguous edit.");
	const call = meta[0]?.initializer;
	const arg = call && ts.isCallExpression(call) && ts.isIdentifier(call.expression) && call.expression.text === "defineMeta" &&
		call.arguments.length === 1 ? call.arguments[0] : undefined;
	const property = arg && ts.isObjectLiteralExpression(arg) && arg.properties.length === 1 ? arg.properties[0] : undefined;
	const array = property && ts.isPropertyAssignment(property) && ts.isIdentifier(property.name) &&
		property.name.text === "capabilities" && ts.isArrayLiteralExpression(property.initializer) ? property.initializer : undefined;
	if (!array || array.elements.some((element) => !ts.isStringLiteral(element))) {
		throw new Error("Computed or nonliteral defineMeta capabilities; refusing ambiguous edit.");
	}
	if (array.elements.some((element) => ts.isStringLiteral(element) && element.text === capability)) return;
	edits.push({ start: array.end - 1, end: array.end - 1,
		text: `${array.elements.hasTrailingComma ? " " : array.elements.length > 0 ? ", " : ""}"${capability}"` });
}
