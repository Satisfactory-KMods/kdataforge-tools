import { Biome } from "@biomejs/js-api/nodejs";

export const biome = new Biome();
export const { projectKey: biomeProjectKey } = biome.openProject();

biome.applyConfiguration(biomeProjectKey, {
	assist: {
		enabled: false,
	},
	formatter: {
		indentStyle: "tab",
		lineEnding: "lf",
		lineWidth: 120,
		trailingNewline: true,
	},
	linter: {
		enabled: true,
		rules: {
			recommended: true,
		},
	},
	json: {
		formatter: {
			expand: "always",
		},
	},
});
