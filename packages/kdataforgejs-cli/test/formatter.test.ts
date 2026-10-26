import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { formatGeneratedContent } from "../src/types/format.js";

describe("generated file formatting", () => {
	it("formats TypeScript declarations through Biome", () => {
		assert.equal(formatGeneratedContent("interface Example{readonly value:string}", "example.d.ts"),
			"interface Example {\n\treadonly value: string;\n}\n");
	});

	it("formats JSON through Biome", () => {
		assert.equal(formatGeneratedContent('{"enabled":true,"items":[1,2]}', "example.json"),
			'{\n\t"enabled": true,\n\t"items": [\n\t\t1,\n\t\t2\n\t]\n}\n');
	});
});
