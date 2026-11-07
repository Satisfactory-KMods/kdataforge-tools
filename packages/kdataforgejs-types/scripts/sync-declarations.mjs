import { copyFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = process.env.KDFJS_RUNTIME_TYPES ?? resolve(root, "../../../../Mods/GameFeatures/KDataForgeJS/Resources/TypeScript");
const copies = [
    ["@sf-kmods/kdataforgejs/index.d.ts", "index.d.ts"], ["lib/kdfjs-es2020.d.ts", "lib/kdfjs-es2020.d.ts"],
    ["kdataforgejs-declarations.json", "declarations.json"], ["editor.kdfjs.json", "editor.kdfjs.json"],
    ["authoring.kdfjs.json", "authoring.kdfjs.json"], ["enums.kdfjs.json", "enums.kdfjs.json"],
];
if (existsSync(join(source, "kdataforgejs-declarations.json"))) {
    for (const [from, to] of copies) {
        mkdirSync(dirname(join(root, to)), { recursive: true });
        copyFileSync(join(source, from), join(root, to));
    }
}
const manifest = JSON.parse(readFileSync(join(root, "declarations.json"), "utf8"));
if (!manifest.sourceArtifacts) throw new Error("SDK snapshot has no integrity manifest; synchronize from the runtime source.");
for (const [, file] of copies) {
    if (!existsSync(join(root, file))) throw new Error(`Missing SDK snapshot: ${file}`);
    if (file === "declarations.json") continue;
    const digest = createHash("sha256").update(readFileSync(join(root, file))).digest("hex");
    if (digest !== manifest.sourceArtifacts[file]) throw new Error(`Stale SDK snapshot: ${file}`);
}
console.log("Verified runtime-pinned KDataForgeJS SDK snapshot");
