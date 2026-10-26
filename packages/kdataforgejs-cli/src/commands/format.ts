import { existsSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { discoverPacks } from "../pack/discover.js";
import { findScriptFiles } from "../pack/scriptFiles.js";
import { commitStaged, stageTexts, withProjectLock } from "../pack/transaction.js";
import { biome, biomeProjectKey } from "../types/biome.js";

export interface FormatProjectResult {
	readonly changedFiles: readonly string[];
}

interface FormattedFile {
	readonly path: string;
	readonly content: string;
}

/** Formats JavaScript and TypeScript files in DataForge/scripts through bundled Biome. */
export function formatProject(projectRoot: string, packRef?: string): FormatProjectResult {
	const absoluteProjectRoot = resolve(projectRoot);
	discoverPacks(absoluteProjectRoot, packRef);
	return withProjectLock(absoluteProjectRoot, () => {
		const scriptsRoots = discoverPacks(absoluteProjectRoot, packRef).packs.map((pack) => join(pack.root, "scripts"));
		for (const scriptsRoot of scriptsRoots) if (!existsSync(scriptsRoot)) throw new Error(`Scripts directory not found: ${scriptsRoot}`);
		const files = new Set(scriptsRoots.flatMap(findScriptFiles));
		const formattedFiles = [...files].sort().map((filePath) => formatFile(filePath, absoluteProjectRoot))
			.filter((file): file is FormattedFile => file !== undefined);
		commitStaged(stageTexts(formattedFiles));
		return { changedFiles: formattedFiles.map((file) => shownPath(file.path, absoluteProjectRoot)) };
	});
}

function formatFile(filePath: string, projectRoot: string): FormattedFile | undefined {
	const original = readFileSync(filePath, "utf8");
	const result = biome.formatContent(biomeProjectKey, original, { filePath });
	if (result.diagnostics.length > 0) {
		const messages = result.diagnostics.map((diagnostic) => diagnostic.description).join("; ");
		throw new Error(`${shownPath(filePath, projectRoot)}: Biome format failed: ${messages}`);
	}
	return result.content === original ? undefined : { path: filePath, content: result.content };
}

function shownPath(filePath: string, projectRoot: string): string {
	return relative(projectRoot, filePath).split(sep).join("/");
}
