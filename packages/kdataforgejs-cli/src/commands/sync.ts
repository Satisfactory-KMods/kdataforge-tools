import { existsSync, readFileSync, realpathSync } from "node:fs";
import { join } from "node:path";
import { isMap, isSeq, parseDocument, YAMLMap, YAMLSeq } from "yaml";
import { discoverPacks, isInside } from "../pack/discover.js";
import { sha256Of } from "../pack/imports.js";
import { assertSafeWrite, commitStaged, stageTexts, withProjectLock } from "../pack/transaction.js";
import { discoverPackUsage, type DiscoveredImport } from "../pack/usage.js";
import { generateConfigTypes } from "../types/configTypes.js";

export interface SyncProjectResult {
	readonly changedPacks: readonly string[];
	readonly changedTypes: readonly string[];
}

export interface PreparedSync {
	readonly result: SyncProjectResult;
	readonly files: readonly { readonly path: string; readonly content: string; readonly expected?: string }[];
}

/** Generates manifest imports, digests, and reflection class authority from source usage. */
export function syncProject(projectRoot: string, packRef?: string): SyncProjectResult {
	discoverPacks(projectRoot, packRef);
	return withProjectLock(projectRoot, () => {
		const prepared = prepareSync(projectRoot, packRef);
		for (const file of prepared.files) {
			const actual = existsSync(file.path) ? readFileSync(file.path, "utf8") : undefined;
			if (actual !== file.expected) throw new Error(`Authored file changed during sync: ${file.path}`);
		}
		commitStaged(stageTexts(prepared.files));
		return prepared.result;
	});
}

/** Reports manifest drift using the same generation path without changing files. */
export function inspectSyncDrift(projectRoot: string, packRef?: string): SyncProjectResult {
	return prepareSync(projectRoot, packRef).result;
}

export function prepareSync(projectRoot: string, packRef?: string): PreparedSync {
	const outputs = discoverPacks(projectRoot, packRef).packs.map((pack) => syncPack(pack.root));
	for (const output of outputs) {
		assertSafeWrite(output.root, output.manifest.path);
		assertSafeWrite(output.root, output.config.path);
	}
	return {
		result: { changedPacks: outputs.filter((output) => output.manifest.changed).map((output) => output.root),
			changedTypes: outputs.filter((output) => output.config.changed).map((output) => output.config.path) },
		files: outputs.flatMap((output) => [output.manifest, output.config].filter((file) => file.changed)
			.map((file) => ({ path: file.path, content: file.content,
				...(existsSync(file.path) ? { expected: readFileSync(file.path, "utf8") } : {}) }))),
	};
}

function syncPack(packRoot: string): { root: string; manifest: Output; config: Output } {
	const manifestFile = join(packRoot, "pack.yml");
	const before = readFileSync(manifestFile, "utf8");
	const document = parseDocument(before);
	const scripts = document.get("scripts", true);
	const config = generateConfigTypes(packRoot);

	for (const script of isSeq(scripts) ? scripts.items : []) {
		if (!isMap(script)) continue;
		const entry = String(script.get("entry") ?? "");
		if (entry === "") continue;
		const usage = discoverPackUsage(packRoot, entry);
		setSequence(script, "reflectionClasses", usage.reflectionClasses);
		setImports(script, packRoot, usage.imports);
	}

	const after = document.toString();
	return { root: packRoot,
		manifest: { path: manifestFile, content: after, changed: after !== before },
		config: { ...config, changed: !existsSync(config.path) || readFileSync(config.path, "utf8") !== config.content } };
}

interface Output { readonly path: string; readonly content: string; readonly changed: boolean }

function setSequence(map: YAMLMap, key: string, values: readonly string[]): void {
	if (values.length === 0) {
		map.delete(key);
		return;
	}
	const sequence = new YAMLSeq();
	sequence.items = [...values];
	map.set(key, sequence);
}

function setImports(script: YAMLMap, packRoot: string, imports: readonly DiscoveredImport[]): void {
	if (imports.length === 0) {
		script.delete("imports");
		return;
	}
	const oldKeys = existingKeys(script);
	const usedKeys = new Set<string>();
	const sequence = new YAMLSeq();
	sequence.items = imports.map((item) => {
		const target = join(packRoot, item.path);
		if (!existsSync(target) || !isInside(packRoot, target) || !isInside(realpathSync(packRoot), realpathSync(target))) {
			throw new Error(`Imported file '${item.path}' does not exist inside ${packRoot}.`);
		}
		const preferred = oldKeys.get(`${item.kind}\n${item.path}`) ?? keyFrom(item);
		const key = uniqueKey(preferred, usedKeys);
		usedKeys.add(key);
		const map = new YAMLMap();
		map.set("key", key);
		map.set("kind", item.kind);
		map.set("path", item.path);
		map.set("sha256", sha256Of(target));
		return map;
	});
	script.set("imports", sequence);
}

function existingKeys(script: YAMLMap): Map<string, string> {
	const result = new Map<string, string>();
	const imports = script.get("imports", true);
	if (!isSeq(imports)) return result;
	for (const item of imports.items) {
		if (!isMap(item)) continue;
		const kind = String(item.get("kind") ?? "");
		const path = String(item.get("path") ?? "");
		const key = String(item.get("key") ?? "");
		if (kind !== "" && path !== "" && key !== "") result.set(`${kind}\n${path}`, key);
	}
	return result;
}

function keyFrom(item: DiscoveredImport): string {
	const normalized = item.path.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
	return normalized === "" ? item.kind : normalized;
}

function uniqueKey(preferred: string, used: ReadonlySet<string>): string {
	if (!used.has(preferred)) return preferred;
	let suffix = 2;
	while (used.has(`${preferred}-${suffix}`)) suffix += 1;
	return `${preferred}-${suffix}`;
}
