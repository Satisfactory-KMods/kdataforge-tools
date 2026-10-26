#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { run } from "./cli.js";

const manifest = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as { version: string };

process.exitCode = await run(
	process.argv.slice(2),
	{ out: (line) => console.log(line), err: (line) => console.error(line) },
	manifest.version,
);
