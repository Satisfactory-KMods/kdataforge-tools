import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { PACK_REF_PATTERN } from "../conventions.js";

export interface InitOptions {
	readonly directory: string;
	readonly name?: string;
	readonly force: boolean;
}

export interface InitResult {
	readonly directory: string;
	readonly ref: string;
}

/** Nearest ancestor holding the CLI's package.json; works from src/, dist/ and compiled tests alike. */
function findCliRoot(directory: string): string {
	return existsSync(join(directory, "package.json")) ? directory : findCliRoot(dirname(directory));
}

const CLI_ROOT = findCliRoot(dirname(fileURLToPath(import.meta.url)));
/** Files that npm would strip or rename on publish are stored with an underscore. */
const RENAMED_ON_COPY: Readonly<Record<string, string>> = { _gitignore: ".gitignore" };

export function packRefFrom(name: string): string {
	const cleaned = name.replace(/[^A-Za-z0-9_-]+/g, "-").replace(/^[^A-Za-z]+/, "").replace(/-+$/, "");
	return cleaned.slice(0, 50);
}

export function initProject(options: InitOptions): InitResult {
	const directory = resolve(options.directory);
	const ref = packRefFrom(options.name ?? basename(directory));
	if (!PACK_REF_PATTERN.test(ref)) {
		throw new Error(`Cannot derive a valid pack ref from '${options.name ?? basename(directory)}'. Use --name with letters, digits, '-' or '_'.`);
	}
	if (existsSync(directory) && readdirSync(directory).length > 0 && !options.force) {
		throw new Error(`Directory '${directory}' is not empty. Use --force to write into it.`);
	}
	mkdirSync(directory, { recursive: true });
	cpSync(join(CLI_ROOT, "templates/default"), directory, { recursive: true });
	for (const [from, to] of Object.entries(RENAMED_ON_COPY)) {
		if (existsSync(join(directory, from))) renameSync(join(directory, from), join(directory, to));
	}
	const values: Readonly<Record<string, string>> = { ref, sdkVersion: readSdkVersion() };
	substitutePlaceholders(directory, values);
	return { directory, ref };
}

function readSdkVersion(): string {
	const manifest = JSON.parse(readFileSync(join(CLI_ROOT, "package.json"), "utf8")) as { version: string };
	return manifest.version;
}

function substitutePlaceholders(directory: string, values: Readonly<Record<string, string>>): void {
	for (const entry of readdirSync(directory, { withFileTypes: true })) {
		const path = join(directory, entry.name);
		if (entry.isDirectory()) {
			substitutePlaceholders(path, values);
			continue;
		}
		const text = readFileSync(path, "utf8");
		const replaced = text.replace(/\{\{(\w+)\}\}/g, (match, key: string) => values[key] ?? match);
		if (replaced !== text) writeFileSync(path, replaced);
	}
}
