const PACK_REF = /^[A-Za-z][A-Za-z0-9_-]{0,49}$/;
const SCRIPT_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,49}$/;

export function projectRef(name: string): string {
  const ref = name.replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^[^A-Za-z]+/, '').replace(/-+$/, '');
  if (!PACK_REF.test(ref))
    throw new Error('Project name must produce a pack ref beginning with a letter and at most 50 characters.');
  return ref;
}

export function scriptFilename(name: string): string {
  const stem = name.replace(/\.ts$/i, '');
  if (!SCRIPT_NAME.test(stem))
    throw new Error('Script name must begin with a letter and contain only letters, digits, "-" or "_".');
  return `${stem}.ts`;
}

export function renderSdkTemplate(content: string, values: Readonly<Record<string, string>>): string {
  return content.replace(/\{\{(\w+)\}\}/g, (match, key: string) => values[key] ?? match);
}

export function newScriptContent(): string {
  return `import { defineHandler } from "@sf-kmods/kdataforgejs";\n\nexport default defineHandler(async (ctx) => {\n    console.info("KDataForgeJS script started", ctx.catalog().digest);\n    await Promise.resolve();\n});\n`;
}
