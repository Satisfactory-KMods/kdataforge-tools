/** Contract shared with the KDataForgeJS runtime (KDataForgeJSModule.cpp). */
export const RUNTIME_MODULE = "@sf-kmods/kdataforgejs";
export const SATISFACTORY_MODULE = "satisfactory";
export const CANONICAL_ENTRY = "scripts/main.ts";
export const SCRIPT_LANGUAGE = "kmods.typescript";
export const REALMS = ["any", "server", "client"] as const;
export const NETWORKS = ["any", "standalone", "networked"] as const;
export const AUTHORITIES = ["any", "required", "forbidden"] as const;
export const IMPORT_KINDS = ["text", "yaml", "structured", "binary", "image", "audio"] as const;
export const MAX_REFLECTION_CLASSES = 64;
/** Capabilities that need a deterministic lane (realm and authority must stay "any"). */
export const DETERMINISTIC_CAPABILITIES = [
	"kdf.items.define",
	"kdf.recipes.define",
	"kdf.documents.apply",
	"kdf.classes.define",
	"kdf.reflection.write",
] as const;
export const PACK_REF_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,49}$/;
export const SCRIPT_ID_PATTERN = /^[a-z0-9][a-z0-9._-]*$/;
