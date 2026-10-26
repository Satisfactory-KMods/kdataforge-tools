import { existsSync, readFileSync, realpathSync } from "node:fs";
import { dirname, extname, relative, resolve, sep } from "node:path";
import ts from "typescript";
import { parse } from "yaml";
import { RUNTIME_MODULE, SATISFACTORY_MODULE } from "../conventions.js";
import { isInside } from "./discover.js";

export interface DiscoveredImport {
	readonly kind: string;
	readonly path: string;
}

export interface PackUsage {
	readonly imports: readonly DiscoveredImport[];
	readonly reflectionClasses: readonly string[];
}

const REFLECTION_METHODS = new Set(["getClass", "getObject", "getDataAsset"]);

/** Discovers manifest authority inputs from reachable TypeScript and imported YAML without executing pack code. */
export function discoverPackUsage(packRoot: string, entry: string): PackUsage {
	const entryFile = resolve(packRoot, entry);
	const scriptsRoot = dirname(entryFile);
	const queue = [entryFile];
	const visited = new Set<string>();
	const imports = new Map<string, DiscoveredImport>();
	const reflectionClasses = new Set<string>();

	while (queue.length > 0) {
		const file = queue.pop() as string;
		if (visited.has(file)) continue;
		visited.add(file);
		if (!existsSync(file)) continue;
		const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ES2020, true);
		visit(source, (node) => {
			const specifier = staticSpecifier(node);
			if (specifier !== undefined) {
				const target = resolveModule(scriptsRoot, file, specifier);
				if (target !== undefined) queue.push(target);
			}
			if (!ts.isCallExpression(node) || !ts.isPropertyAccessExpression(node.expression)) return;
			const method = node.expression.name.text;
			if (method === "import") {
				const kind = stringLiteral(node.arguments[0]);
				const path = stringLiteral(node.arguments[1]);
				if (kind === undefined || path === undefined) {
					throw new Error(`${shownPath(packRoot, file)}: ctx.import requires literal kind and path so CLI can secure manifest imports.`);
				}
				imports.set(`${kind}\n${path}`, { kind, path });
			} else if (REFLECTION_METHODS.has(method)) {
				const path = stringLiteral(node.arguments[1]);
				if (path !== undefined) reflectionClasses.add(path);
			}
		});
	}

	for (const item of imports.values()) {
		if (item.kind !== "yaml") continue;
		const target = resolveInside(packRoot, item.path);
		if (target === undefined || !existsSync(target)) continue;
		collectYamlTargets(parse(readFileSync(target, "utf8")), reflectionClasses);
	}

	return {
		imports: [...imports.values()].sort((left, right) => left.path.localeCompare(right.path) || left.kind.localeCompare(right.kind)),
		reflectionClasses: [...reflectionClasses].sort((left, right) => left.localeCompare(right)),
	};
}

function visit(node: ts.Node, callback: (node: ts.Node) => void): void {
	callback(node);
	ts.forEachChild(node, (child) => visit(child, callback));
}

function staticSpecifier(node: ts.Node): string | undefined {
	if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier !== undefined &&
		ts.isStringLiteral(node.moduleSpecifier)) return node.moduleSpecifier.text;
	return undefined;
}

function resolveModule(scriptsRoot: string, importer: string, specifier: string): string | undefined {
	if (specifier === RUNTIME_MODULE || specifier === SATISFACTORY_MODULE ||
		(!specifier.startsWith("./") && !specifier.startsWith("../"))) return undefined;
	const extension = extname(specifier);
	const target = resolve(dirname(importer), extension === "" ? `${specifier}.ts` : specifier);
	if (!isInside(scriptsRoot, target) || (existsSync(target) && !isInside(realpathSync(scriptsRoot), realpathSync(target)))) {
		throw new Error(`Script import escapes scripts/: ${specifier}`);
	}
	return target;
}

function stringLiteral(node: ts.Expression | undefined): string | undefined {
	return node !== undefined && (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) ? node.text : undefined;
}

function resolveInside(packRoot: string, path: string): string | undefined {
	const target = resolve(packRoot, path);
	return isInside(packRoot, target) && (!existsSync(target) || isInside(realpathSync(packRoot), realpathSync(target))) ? target : undefined;
}

function collectYamlTargets(value: unknown, targets: Set<string>): void {
	if (Array.isArray(value)) {
		for (const item of value) collectYamlTargets(item, targets);
		return;
	}
	if (typeof value !== "object" || value === null) return;
	for (const [key, child] of Object.entries(value)) {
		if (key === "target" && typeof child === "string" && child.startsWith("/")) targets.add(child);
		collectYamlTargets(child, targets);
	}
}

function shownPath(packRoot: string, file: string): string {
	return relative(packRoot, file).split(sep).join("/");
}
