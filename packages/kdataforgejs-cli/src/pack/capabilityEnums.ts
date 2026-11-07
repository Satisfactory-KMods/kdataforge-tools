import ts from "typescript";
import { apiEnums } from "../types/apiEnums.js";

/** Resolve only literals and direct, uniquely imported SDK Capability members. Never evaluate user code. */
export function capabilityLiteral(value: ts.Expression, source: ts.SourceFile): string | undefined {
	return sdkEnumLiteral(value, source, "Capability");
}

export function sdkEnumLiteral(value: ts.Expression, source: ts.SourceFile, enumName: string): string | undefined {
	if (ts.isStringLiteral(value)) return value.text;
	if (!ts.isPropertyAccessExpression(value) || !ts.isIdentifier(value.expression)) return undefined;
	const alias = value.expression.text;
	let bindings = 0;
	let capabilityImport = false;
	function count(name: ts.BindingName): void {
		if (ts.isIdentifier(name)) { if (name.text === alias) ++bindings; }
		else for (const element of name.elements) if (ts.isBindingElement(element)) count(element.name);
	}
	for (const statement of source.statements) {
		if (ts.isImportDeclaration(statement) && statement.importClause) {
			const clause = statement.importClause;
			if (clause.name) count(clause.name);
			if (clause.namedBindings) {
				if (ts.isNamespaceImport(clause.namedBindings)) count(clause.namedBindings.name);
				else for (const element of clause.namedBindings.elements) {
					count(element.name);
					if (element.name.text === alias && !clause.isTypeOnly && !element.isTypeOnly &&
						ts.isStringLiteral(statement.moduleSpecifier) && statement.moduleSpecifier.text === "@sf-kmods/kdataforgejs" &&
						(element.propertyName ?? element.name).text === enumName) capabilityImport = true;
				}
			}
		} else if (ts.isVariableStatement(statement)) {
			for (const declaration of statement.declarationList.declarations) count(declaration.name);
		} else if ((ts.isFunctionDeclaration(statement) || ts.isClassDeclaration(statement) || ts.isEnumDeclaration(statement)) && statement.name) count(statement.name);
	}
	const enumMembers = apiEnums.enums.find((entry) => entry.name === enumName)?.members;
	// Import use within callbacks must not resolve through a local shadow.
	let shadowed = false;
	const visit = (node: ts.Node): void => {
		if (ts.isParameter(node) || ts.isVariableDeclaration(node)) {
			const names = (name: ts.BindingName): boolean => ts.isIdentifier(name) ? name.text === alias : name.elements.some((entry) => ts.isBindingElement(entry) && names(entry.name));
			if (names(node.name)) shadowed = true;
		}
		ts.forEachChild(node, visit);
	};
	if (enumName !== "Capability") visit(source);
	const result = bindings === 1 && !shadowed && capabilityImport && enumMembers && Object.hasOwn(enumMembers, value.name.text) ? enumMembers[value.name.text] : undefined;
	return typeof result === "string" ? result : undefined;
}
