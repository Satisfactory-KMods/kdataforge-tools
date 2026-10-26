import { readdirSync } from "node:fs";
import { extname, join } from "node:path";

const SCRIPT_EXTENSIONS = new Set([".js", ".ts"]);

export function findScriptFiles(directory: string): string[] {
	const files: string[] = [];
	for (const entry of readdirSync(directory, { withFileTypes: true })) {
		const path = join(directory, entry.name);
		if (entry.isDirectory()) files.push(...findScriptFiles(path));
		else if (entry.isFile() && SCRIPT_EXTENSIONS.has(extname(entry.name).toLocaleLowerCase())) files.push(path);
	}
	return files.sort((left, right) => left.localeCompare(right));
}
