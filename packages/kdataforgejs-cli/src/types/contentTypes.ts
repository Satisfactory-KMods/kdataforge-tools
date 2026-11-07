import { existsSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { parse } from "yaml";
import { apiEnums, enumMember } from "./apiEnums.js";

type Schema = Readonly<Record<string, unknown>>;
interface LocatedSchema { readonly file: string; readonly pointer: string; readonly schema: Schema; }

const DOCUMENTS: Readonly<Record<string, string>> = {
	AssetDocument: "asset", CategoryReorderDocument: "category_reorder", CdoDocument: "cdo",
	CurveDocument: "curve", DataAssetDocument: "dataasset", EventRuleDocument: "eventrule",
	GameTagDocument: "gametag", ItemDocument: "item", ResourceDocument: "resource", BuildingDocument: "building",
	UnlockDocument: "unlock", RecipeDocument: "recipe", SchematicDocument: "schematic", ResearchDocument: "research",
	ResourceNodeDocument: "resourcenode", SinkPointsDocument: "sinkpoints", SoundDocument: "sound", SublevelDocument: "sublevel",
};

/** Uses the same bundled schemas as KDF validation; never resolves schema URLs over the network. */
export function strictContentDeclarations(source: string): string {
	const generator = new ContentTypes();
	const file = ts.createSourceFile("runtime.d.ts", source, ts.ScriptTarget.Latest, true);
	const replacements: Array<{ start: number; end: number; text: string }> = [];
	const visit = (node: ts.Node): void => {
		if (ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node)) {
			const schema = DOCUMENTS[node.name.text];
			if (schema !== undefined) replacements.push({ start: node.getStart(file), end: node.end,
				text: `export type ${node.name.text} = ${generator.reference(`${schema}.schema.yml`, "")};` });
		}
		ts.forEachChild(node, visit);
	};
	visit(file);
	let result = source;
	for (const replacement of replacements.sort((left, right) => right.start - left.start)) {
		result = result.slice(0, replacement.start) + replacement.text + result.slice(replacement.end);
	}
	if (replacements.length === 0) return source;
	return `${result}\ndeclare module "@sf-kmods/kdataforgejs" {\n${generator.declarations()}\n}\n`;
}

class ContentTypes {
	private readonly schemas = new Map<string, Schema>();
	private readonly aliases = new Map<string, LocatedSchema>();
	private readonly directory = schemaDirectory();

	reference(file: string, pointer: string): string {
		if (!/^[a-z_\-]+\.schema\.yml$/u.test(file) || (pointer !== "" && !pointer.startsWith("/"))) {
			throw new Error(`Unsupported local schema reference: ${file}#${pointer}`);
		}
		const key = `${file}#${pointer}`;
		const name = aliasName(key);
		if (!this.aliases.has(key)) {
			let node: unknown = this.load(file);
			for (const part of pointer.split("/").slice(1)) {
				node = record(node)?.[part.replaceAll("~1", "/").replaceAll("~0", "~")];
			}
			const schema = record(node);
			if (schema === undefined) throw new Error(`Missing bundled schema definition: ${key}`);
			this.aliases.set(key, { file, pointer, schema });
		}
		return name;
	}

	declarations(): string {
		const declarations: string[] = [];
		for (const [key, located] of this.aliases) {
			declarations.push(`type ${aliasName(key)} = ${this.render(located)};`);
		}
		return declarations.join("\n");
	}

	private load(file: string): Schema {
		const cached = this.schemas.get(file);
		if (cached !== undefined) return cached;
		const schema = record(parse(readFileSync(join(this.directory, file), "utf8")));
		if (schema === undefined) throw new Error(`Invalid bundled schema: ${file}`);
		this.schemas.set(file, schema);
		return schema;
	}

	private render(located: LocatedSchema): string {
		const { schema, file, pointer } = located;
		const parts: string[] = [];
		if (typeof schema.$ref === "string") {
			const [path = "", pointer = ""] = schema.$ref.split("#");
			parts.push(this.reference(path === "" ? file : basename(path), pointer));
		}
		const binding = apiEnums.schemaBindings.find((entry) => entry.file === file && entry.pointer === pointer);
		if ("const" in schema) parts.push(binding ? enumMember(binding.enum, schema.const) : literal(schema.const));
		else if (Array.isArray(schema.enum)) {
			if (!binding) throw new Error(`Unmapped schema enum: ${file}#${pointer}`);
			parts.push(schema.enum.map((value) => enumMember(binding.enum, value)).join(" | "));
		}
		else {
			const type = this.valueType(located);
			if (type !== undefined) parts.push(type);
		}
		for (const key of ["allOf", "anyOf", "oneOf"]) {
			const entries = schema[key];
			if (!Array.isArray(entries)) continue;
			const types = entries.map((entry, index) => this.render({ file, pointer: `${pointer}/${key}/${index}`, schema: record(entry) ?? {} }));
			if (types.length > 0) parts.push(`(${types.join(key === "allOf" ? " & " : " | ")})`);
		}
		return parts.length === 0 ? "unknown" : parts.map((part) => `(${part})`).join(" & ");
	}

	private valueType(located: LocatedSchema): string | undefined {
		const { schema, file, pointer } = located;
		if (Array.isArray(schema.type)) {
			return schema.type.map((type) => this.valueType({ ...located, schema: { ...schema, type } }) ?? "unknown").join(" | ");
		}
		if (schema.type === "string") return "string";
		if (schema.type === "number" || schema.type === "integer") return "number";
		if (schema.type === "boolean") return "boolean";
		if (schema.type === "null") return "null";
		if (schema.type === "array") return `ReadonlyArray<${this.render({ file, pointer: `${pointer}/items`, schema: record(schema.items) ?? {} })}>`;
		if (schema.type === "object" || schema.properties !== undefined || schema.required !== undefined) return this.objectType(located);
		return undefined;
	}

	private objectType({ schema, file, pointer }: LocatedSchema): string {
		const properties = record(schema.properties) ?? {};
		const required = new Set(Array.isArray(schema.required) ? schema.required.filter((value): value is string => typeof value === "string") : []);
		const names = [...new Set([...Object.keys(properties), ...required])].sort();
		const fields = names.map((name) => `readonly ${JSON.stringify(name)}${required.has(name) ? "" : "?"}: ${this.render({ file, pointer: `${pointer}/properties/${name.replaceAll("~", "~0").replaceAll("/", "~1")}`, schema: record(properties[name]) ?? {} })};`);
		const excluded = record(schema.not)?.required;
		if (Array.isArray(excluded) && excluded.length === 1 && typeof excluded[0] === "string") {
			fields.push(`readonly ${JSON.stringify(excluded[0])}?: never;`);
		}
		const additional = record(schema.additionalProperties);
		if (additional !== undefined) fields.push(`readonly [key: string]: ${this.render({ file, pointer: `${pointer}/additionalProperties`, schema: additional })};`);
		if (fields.length === 0 && schema.additionalProperties !== false) return "Readonly<Record<string, unknown>>";
		return `{ ${fields.join(" ")} }`;
	}
}

function record(value: unknown): Schema | undefined {
	return typeof value === "object" && value !== null && !Array.isArray(value) ? value as Schema : undefined;
}

function literal(value: unknown): string {
	if (value === null || typeof value === "string" || typeof value === "number" || typeof value === "boolean") return JSON.stringify(value);
	throw new Error("Content schema contains a non-scalar enum or const.");
}

function aliasName(key: string): string {
	return `KDFSchema_${key.replace(/\.schema\.yml/gu, "").replace(/[^a-zA-Z0-9]/gu, "_")}`;
}

function schemaDirectory(): string {
	const directories = [new URL("../../schemas/", import.meta.url), new URL("../../../schemas/", import.meta.url)];
	const directory = directories.find((candidate) => existsSync(new URL("common.schema.yml", candidate)));
	if (directory === undefined) throw new Error("Bundled content schemas missing; rebuild the CLI package.");
	return fileURLToPath(directory);
}
