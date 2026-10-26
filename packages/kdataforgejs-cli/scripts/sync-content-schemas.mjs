import { copyFileSync, mkdirSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const source = new URL("../../../crates/kdf-validation/schemas/", import.meta.url);
const destination = new URL("../schemas/", import.meta.url);
mkdirSync(destination, { recursive: true });
for (const name of readdirSync(source).filter((entry) => entry.endsWith(".schema.yml")).sort()) {
	copyFileSync(new URL(name, source), new URL(name, destination));
}
console.log(`Synchronized content schemas into ${fileURLToPath(destination)}`);
