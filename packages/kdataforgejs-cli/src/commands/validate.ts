import { existsSync } from "node:fs";
import { join } from "node:path";
import { CANONICAL_ENTRY, DETERMINISTIC_CAPABILITIES } from "../conventions.js";
import type { Diagnostic } from "../diagnostics.js";
import { discoverPacks } from "../pack/discover.js";
import { checkImports } from "../pack/imports.js";
import { readManifest } from "../pack/manifest.js";
import { checkEntryShape, checkSourceGraph, readEntryMeta } from "../pack/sourceGraph.js";

/** Static checks mirroring what the KDataForgeJS runtime rejects at load time. */
export function validateProject(projectRoot: string, packRef?: string): Diagnostic[] {
	try {
		const selection = discoverPacks(projectRoot, packRef);
		return selection.packs.flatMap((pack) => validatePack(pack.root).map((diagnostic) => ({
			...diagnostic,
			file: selection.packs.length > 1 ? `${pack.ref}/${diagnostic.file}` : diagnostic.file,
		})));
	} catch (error) {
		return [{ severity: "error", code: "project.discovery", file: projectRoot,
			message: error instanceof Error ? error.message : String(error) }];
	}
}

function validatePack(packRoot: string): Diagnostic[] {
	const manifest = readManifest(packRoot);
	const diagnostics = [...manifest.diagnostics];
	for (const forbidden of ["package.json", "node_modules"]) {
		if (existsSync(join(packRoot, forbidden))) {
			diagnostics.push({ severity: "error", code: "pack.forbidden", file: forbidden, message: `Packs must not contain '${forbidden}'; keep it in the project root.` });
		}
	}
	for (const script of manifest.scripts) {
		if (!existsSync(join(packRoot, CANONICAL_ENTRY))) {
			diagnostics.push({ severity: "error", code: "entry.missing", file: CANONICAL_ENTRY, message: "Entry file does not exist." });
			continue;
		}
		const meta = readEntryMeta(packRoot, script.entry);
		diagnostics.push(...meta.diagnostics);
		const deterministic = meta.capabilities.some((capability) =>
			(DETERMINISTIC_CAPABILITIES as readonly string[]).includes(capability));
		if (deterministic && (script.realm !== "any" || script.authority !== "any")) {
			diagnostics.push({ severity: "error", code: "script.deterministic-lane", file: script.entry,
				message: "Authoring capabilities require realm: any and authority: any." });
		}
		diagnostics.push(
			...checkEntryShape(packRoot, script.entry),
			...checkSourceGraph(packRoot, script.entry),
			...checkImports(packRoot, [script]),
		);
	}
	return diagnostics;
}
