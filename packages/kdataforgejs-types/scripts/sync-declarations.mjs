import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const required = [
	"index.d.ts",
	"lib/kdfjs-es2020.d.ts",
	"declarations.json",
	"editor.kdfjs.json",
	"tsconfig.base.json",
];
for (const file of required) {
	if (!existsSync(join(root, file))) throw new Error(`Missing public SDK snapshot file: ${file}`);
}
if (!readFileSync(join(root, "index.d.ts"), "utf8").includes('declare module "@sf-kmods/kdataforgejs"'))
	throw new Error("KDataForgeJS module declaration is missing");
console.log("Verified bundled KDataForgeJS declaration snapshot");
