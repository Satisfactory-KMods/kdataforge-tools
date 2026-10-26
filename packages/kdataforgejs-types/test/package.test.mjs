import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const base = JSON.parse(readFileSync(join(root, "tsconfig.base.json"), "utf8"));

describe("@sf-kmods/kdataforgejs", () => {
	it("declares the module under the package name", () => {
		const declaration = readFileSync(join(root, "index.d.ts"), "utf8");
		assert.ok(declaration.includes(`declare module "${manifest.name}"`));
	});
	it("ships every published file", () => {
		for (const entry of manifest.files) assert.ok(existsSync(join(root, entry)), entry);
	});
	it("references only files that exist in tsconfig.base.json", () => {
		for (const file of base.compilerOptions.noLib ? base.files : []) assert.ok(existsSync(join(root, file)), file);
	});
	it("exposes the tsconfig base", () => {
		assert.equal(manifest.exports["./tsconfig.base.json"], "./tsconfig.base.json");
	});
});
