import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, describe, it } from "node:test";
import { syncConfigTypes } from "../src/types/configTypes.js";

const roots: string[] = [];
after(() => { for (const root of roots) rmSync(root, { recursive: true, force: true }); });

function pack(): string {
	const root = mkdtempSync(join(tmpdir(), "kdf-config-types-"));
	roots.push(root);
	mkdirSync(join(root, "settings"));
	return root;
}

describe("pack config declarations", () => {
	it("derives scalar and array types from config definitions", () => {
		const root = pack();
		writeFileSync(join(root, "settings", "balance.config.yml"), `type: config\nfields:\n  - {id: enabled, type: boolean, default: true}\n  - {id: rate, type: number, default: 2}\n  - {id: total, type: number, valueType: int64, default: 2}\n  - {id: labels, type: text, multiple: true, default: [x]}\n`);
		const path = syncConfigTypes(root);
		const text = readFileSync(path, "utf8");
		assert.match(text, /readonly enabled: boolean/);
		assert.match(text, /readonly rate: number/);
		assert.match(text, /readonly total: number \| bigint/);
		assert.match(text, /readonly labels: readonly string\[\]/);
	});
	it("rejects malformed pack-local ids", () => {
		const root = pack();
		writeFileSync(join(root, "settings", "invalid.config.yml"), `type: config\nfields:\n  - {id: "bad:id", type: boolean, default: true}\n`);
		assert.throws(() => syncConfigTypes(root), /config id/);
	});
	it("uses declared select valueType rather than option literal types", () => {
		const root = pack();
		writeFileSync(join(root, "select.config.yml"), `fields:\n  - {id: numericText, type: select, options: [{value: 2, label: Two}], default: 2}\n  - {id: numeric, type: select, valueType: int32, options: [{value: 2, label: Two}], default: 2}\n`);
		const output = readFileSync(syncConfigTypes(root), "utf8");
		assert.match(output, /readonly numericText: string/);
		assert.match(output, /readonly numeric: number/);
	});
	it("rejects duplicate config ids", () => {
		const root = pack();
		writeFileSync(join(root, "duplicate.config.yml"), `fields:\n  - {id: flag, type: boolean}\n  - {id: flag, type: text}\n`);
		assert.throws(() => syncConfigTypes(root), /Duplicate config id/);
	});
});
