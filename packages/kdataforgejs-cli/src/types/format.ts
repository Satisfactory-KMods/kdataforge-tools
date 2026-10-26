import { biome, biomeProjectKey } from "./biome.js";

/** Formats generated TypeScript and JSON through Biome's in-process JavaScript API. */
export function formatGeneratedContent(content: string, filePath: string): string {
	const formatted = biome.formatContent(biomeProjectKey, content, { filePath });
	if (formatted.diagnostics.length > 0) {
		throw new Error(`Biome could not format generated file '${filePath}'.`);
	}
	return formatted.content;
}
