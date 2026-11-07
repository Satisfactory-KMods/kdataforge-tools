import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

interface ApiEnum { readonly name: string; readonly members: Readonly<Record<string, string | number>> }
interface SchemaBinding { readonly file: string; readonly pointer: string; readonly enum: string }
interface EnumDescription { readonly enums: readonly ApiEnum[]; readonly schemaBindings: readonly SchemaBinding[] }
const require = createRequire(import.meta.url);
const raw: unknown = JSON.parse(readFileSync(require.resolve("@sf-kmods/kdataforgejs/enums.kdfjs.json"), "utf8"));
function record(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}
if (!record(raw) || !Array.isArray(raw.enums) || !Array.isArray(raw.schemaBindings) ||
	raw.enums.some((value: unknown) => !record(value) || typeof value.name !== "string" || !record(value.members) ||
		Object.values(value.members).some((member) => typeof member !== "string" && !Number.isSafeInteger(member))) ||
	raw.schemaBindings.some((value: unknown) => !record(value) || typeof value.file !== "string" || typeof value.pointer !== "string" || typeof value.enum !== "string")) {
	throw new Error("Invalid canonical SDK enum description.");
}
export const apiEnums = raw as unknown as EnumDescription;

export function enumMember(name: string, value: unknown): string {
	const entry = apiEnums.enums.find((entry) => entry.name === name);
	const member = entry && Object.entries(entry.members).find(([, candidate]) => candidate === value)?.[0];
	if (member === undefined) throw new Error(`Unknown ${name} enum value '${String(value)}'; synchronize the SDK and schemas.`);
	return `${name}.${member}`;
}
