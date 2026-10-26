import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, join } from "node:path";
import { isScalar, parseDocument, type Document } from "yaml";
import {
	AUTHORITIES,
	CANONICAL_ENTRY,
	IMPORT_KINDS,
	MAX_REFLECTION_CLASSES,
	NETWORKS,
	PACK_REF_PATTERN,
	REALMS,
	SCRIPT_LANGUAGE,
} from "../conventions.js";
import type { Diagnostic } from "../diagnostics.js";

export interface ScriptImport {
	readonly key: string;
	readonly kind: string;
	readonly path: string;
	readonly sha256: string;
}

export interface ScriptEntry {
	readonly id: string;
	readonly entry: string;
	readonly realm: string;
	readonly authority: string;
	readonly imports: readonly ScriptImport[];
}

export interface PackManifest {
	readonly ref: string | undefined;
	readonly scripts: readonly ScriptEntry[];
	readonly diagnostics: Diagnostic[];
}

type Json = Record<string, unknown>;
type Fail = (code: string, message: string) => void;

const isRecord = (value: unknown): value is Json => typeof value === "object" && value !== null && !Array.isArray(value);
const oneOf = (allowed: readonly string[], value: unknown): boolean =>
	typeof value === "string" && allowed.includes(value.toLowerCase());

export function readManifest(packRoot: string): PackManifest {
	const file = "pack.yml";
	const diagnostics: Diagnostic[] = [];
	const fail: Fail = (code, message) => {
		diagnostics.push({ severity: "error", code, file, message });
	};
	const document = parseDocument(readFileSync(join(packRoot, file), "utf8"));
	for (const error of document.errors) fail("manifest.yaml", error.message);
	const root: unknown = document.toJS();
	if (!isRecord(root)) {
		fail("manifest.shape", "pack.yml must be a map.");
		return { ref: undefined, scripts: [], diagnostics };
	}
	const ref = typeof root.ref === "string" ? root.ref : basename(dirname(packRoot));
	if (ref !== undefined && !PACK_REF_PATTERN.test(ref)) fail("manifest.ref", `Pack ref '${ref}' is not a safe ref.`);
	const rawScripts = root.scripts ?? [];
	if (!Array.isArray(rawScripts)) {
		fail("manifest.scripts", "`scripts` must be a sequence.");
		return { ref, scripts: [], diagnostics };
	}
	const scripts = rawScripts.flatMap((raw, index) => parseScript(raw, index, ref, fail, document));
	return { ref, scripts, diagnostics };
}

function parseScript(raw: unknown, index: number, ref: string | undefined, fail: Fail, document: Document): ScriptEntry[] {
	const at = `scripts[${index}]`;
	if (!isRecord(raw)) {
		fail("script.shape", `${at} must be a map.`);
		return [];
	}
	if (raw.id !== undefined) fail("script.id", `${at}.id is generated from pack ref and entry; remove it.`);
	if (raw.language !== SCRIPT_LANGUAGE) fail("script.language", `${at}.language must be '${SCRIPT_LANGUAGE}'.`);
	const version = raw.languageVersion ?? 1;
	if (!Number.isInteger(version) || (version as number) <= 0) {
		fail("script.language-version", `${at}.languageVersion must be a positive integer.`);
	}
	const entry = typeof raw.entry === "string" ? raw.entry : "";
	if (entry !== CANONICAL_ENTRY) fail("script.entry", `${at}.entry must be '${CANONICAL_ENTRY}'.`);
	const id = `script.${createHash("sha256").update(`${ref ?? ""}\n${entry}`, "utf8").digest("hex")}`;
	const policies: ReadonlyArray<readonly [string, readonly string[]]> = [
		["realm", REALMS],
		["network", NETWORKS],
		["authority", AUTHORITIES],
	];
	for (const [key, allowed] of policies) {
		if (raw[key] !== undefined && !oneOf(allowed, raw[key])) {
			fail(`script.${key}`, `${at}.${key} must be ${allowed.join(", ")}.`);
		}
	}
	if (raw.capabilities !== undefined) fail("script.capabilities", `${at}.capabilities belongs in optional TypeScript meta.`);
	checkReflectionClasses(raw, at, fail);
	return [{ id, entry, realm: String(raw.realm ?? "any").toLowerCase(), authority: String(raw.authority ?? "any").toLowerCase(),
		imports: parseImports(raw.imports, at, fail, (i) => rawDigest(document, index, i)) }];
}

function checkReflectionClasses(raw: Json, at: string, fail: Fail): void {
	if (raw.reflectionClasses !== undefined && raw.classes !== undefined) {
		fail("script.classes", `${at}: reflectionClasses and classes cannot both be present.`);
	}
	const classes = raw.reflectionClasses ?? raw.classes;
	if (classes === undefined) return;
	if (!Array.isArray(classes) || classes.length > MAX_REFLECTION_CLASSES) {
		fail("script.classes", `${at}.reflectionClasses must be a sequence with at most ${MAX_REFLECTION_CLASSES} entries.`);
		return;
	}
	for (const path of classes) {
		if (typeof path !== "string" || !path.startsWith("/") || path.includes("\\") || path.includes("..") || path.length > 1024) {
			fail("script.classes", `${at}: invalid reflection class path '${String(path)}'.`);
		}
	}
	if (new Set(classes).size !== classes.length) fail("script.classes", `${at}: reflection class paths must be unique.`);
}

function parseImports(raw: unknown, at: string, fail: Fail, digestAt: (importIndex: number) => string): ScriptImport[] {
	if (raw === undefined) return [];
	if (!Array.isArray(raw)) {
		fail("script.imports", `${at}.imports must be a sequence.`);
		return [];
	}
	const imports: ScriptImport[] = [];
	for (const [index, item] of raw.entries()) {
		if (!isRecord(item) || typeof item.key !== "string" || typeof item.path !== "string") {
			fail("script.import", `${at}.imports[${index}] needs string key and path.`);
			continue;
		}
		const kind = typeof item.kind === "string" ? item.kind : "";
		if (!oneOf(IMPORT_KINDS, kind)) fail("script.import-kind", `${at}.imports[${index}].kind '${kind}' is unsupported.`);
		const sha256 = digestAt(index);
		imports.push({ key: item.key, kind, path: item.path, sha256 });
	}
	if (new Set(imports.map((i) => i.key)).size !== imports.length) fail("script.import-key", `${at}: import keys must be unique.`);
	return imports;
}

/** A digest made only of digits would load as a YAML number, so the digest is read as written. */
function rawDigest(document: Document, scriptIndex: number, importIndex: number): string {
	for (const key of ["sha256", "digest"]) {
		const node = document.getIn(["scripts", scriptIndex, "imports", importIndex, key], true);
		if (isScalar(node)) return node.source ?? String(node.value);
	}
	return "";
}
