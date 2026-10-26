import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { isInside } from "./discover.js";
import { isMap, isSeq, parseDocument } from "yaml";
import type { Diagnostic } from "../diagnostics.js";
import type { ScriptEntry } from "./manifest.js";

const SHA256_PATTERN = /^[0-9a-f]{64}$/;

export function sha256Of(file: string): string {
	return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function resolveInside(packRoot: string, path: string): string | undefined {
	const target = resolve(packRoot, path);
	return isInside(packRoot, target) && (!existsSync(target) || isInside(realpathSync(packRoot), realpathSync(target))) ? target : undefined;
}

export function checkImports(packRoot: string, scripts: readonly ScriptEntry[]): Diagnostic[] {
	const diagnostics: Diagnostic[] = [];
	const report = (code: string, message: string): void => {
		diagnostics.push({ severity: "error", code, file: "pack.yml", message });
	};
	for (const script of scripts) {
		for (const item of script.imports) {
			const target = resolveInside(packRoot, item.path);
			if (target === undefined || !existsSync(target)) {
				report("import.missing", `Import '${item.key}': '${item.path}' does not exist inside the pack.`);
			} else if (!SHA256_PATTERN.test(item.sha256)) {
				report("import.digest", `Import '${item.key}': sha256 is missing or malformed. Run 'kdf hash'.`);
			} else if (sha256Of(target) !== item.sha256) {
				report("import.digest-mismatch", `Import '${item.key}': sha256 does not match '${item.path}'. Run 'kdf hash'.`);
			}
		}
	}
	return diagnostics;
}

/** Rewrites stale or missing sha256 values in place and keeps comments and formatting. Returns changed keys. */
export function syncImportDigests(packRoot: string): string[] {
	const update = updatedImportDigests(packRoot);
	if (update.changed.length > 0) writeFileSync(join(packRoot, "pack.yml"), update.content);
	return update.changed;
}

export function updatedImportDigests(packRoot: string): { readonly content: string; readonly changed: string[] } {
	const manifestFile = join(packRoot, "pack.yml");
	const document = parseDocument(readFileSync(manifestFile, "utf8"));
	const scripts = document.get("scripts");
	const changed: string[] = [];
	if (isSeq(scripts)) {
		for (const script of scripts.items) {
			const imports = isMap(script) ? script.get("imports") : undefined;
			if (!isSeq(imports)) continue;
			for (const item of imports.items) {
				if (!isMap(item)) continue;
				const target = resolveInside(packRoot, String(item.get("path")));
				if (target === undefined || !existsSync(target)) continue;
				const digest = sha256Of(target);
				if (item.get("sha256") !== digest) {
					item.set("sha256", digest);
					changed.push(String(item.get("key")));
				}
			}
		}
	}
	return { content: document.toString(), changed };
}
