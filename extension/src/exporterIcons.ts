const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};

/**
 * Exporter icon contract, KDataForgeReflection commit 3a16e5ae0. Paths remain index keys.
 * `property`/`smallProperty` name the reflected property holding each texture on `via.class`
 * (or on the class itself for `self`); exports older than that field omit them.
 */
export interface ExporterIcon {
  texture: string;
  image?: string;
  property?: string;
  smallTexture?: string;
  smallImage?: string;
  smallProperty?: string;
  source: 'self' | 'descriptor' | 'product' | 'schematic';
  via?: { class: string; property: string; alternatives?: number };
}

const MAX_PROPERTY_NAME_LENGTH = 256;
const propertyName = (value: unknown): value is string =>
  typeof value === 'string' &&
  value.length > 0 &&
  value.length <= MAX_PROPERTY_NAME_LENGTH &&
  !/[\u0000-\u001f]/.test(value);

/** ExportText object aliases retain the exact asset path; never append _C or infer a basename. */
export function exportedClassReference(value: unknown): string | null {
  if (value && typeof value === 'object' && !Array.isArray(value))
    return exportedClassReference(record(value).path);
  if (typeof value !== 'string' || value.length > 4096) return null;
  let text = value.trim();
  if (text.startsWith('"') && text.endsWith('"')) {
    try {
      text = JSON.parse(text) as string;
    } catch {
      return null;
    }
  }
  const alias = text.match(/^[A-Za-z0-9_./]+'([^']+)'$/);
  const path = alias?.[1] ?? text;
  if (
    !/^\/[^\s'"()[\],=\\\u0000-\u001f]+$/.test(path) ||
    path.split('/').some((part) => part === '..' || part === '.')
  )
    return null;
  return path;
}

type ItemHead = { state: 'empty' } | { state: 'unknown' } | { state: 'first'; path: string | null };
/** Split native ExportText at a top-level comma, honoring nested structs and quoted aliases. */
function nativeHead(text: string): string | null {
  let depth = 0;
  let quote = '';
  for (let index = 0; index < text.length; index++) {
    const char = text[index]!;
    if (quote) {
      if (char === '\\') index++;
      else if (char === quote) quote = '';
    } else if (char === '"' || char === "'") quote = char;
    else if (char === '(' || char === '[' || char === '{') {
      if (++depth > 64) return null;
    } else if (char === ')' || char === ']' || char === '}') {
      if (--depth < 0) return null;
    } else if (char === ',' && depth === 0) return text.slice(0, index).trim();
  }
  return depth === 0 && !quote ? text.trim() : null;
}
export function firstRecipeItem(value: unknown): ItemHead {
  if (Array.isArray(value)) return value.length ? firstRecipeItem(value[0]) : { state: 'empty' };
  if (value && typeof value === 'object')
    return { state: 'first', path: exportedClassReference(record(value).ItemClass) };
  if (typeof value !== 'string' || value.length > 65_536) return { state: 'unknown' };
  const text = value.trim();
  if (text === '()') return { state: 'empty' };
  if (text.startsWith('[') || text.startsWith('{')) {
    try {
      return firstRecipeItem(JSON.parse(text));
    } catch {
      return { state: 'unknown' };
    }
  }
  if (!text.startsWith('(') || !text.endsWith(')')) return { state: 'unknown' };
  const outer = text.slice(1, -1).trim();
  const head = nativeHead(outer);
  if (head === null) return { state: 'unknown' };
  const struct = head.startsWith('(') && head.endsWith(')') ? head.slice(1, -1) : outer;
  // ItemClass must be a field on the first struct. Later product rows never enter this search.
  const fields = struct.match(
    /(?:^|,)\s*ItemClass\s*=\s*("(?:[^"\\]|\\.)*"|[A-Za-z0-9_./]+'[^']*'|[^,()]*)/,
  );
  return { state: 'first', path: exportedClassReference(fields?.[1]) };
}

export function safeIconImage(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length <= 1024 &&
    /^images\/[A-Za-z0-9_./ -]+\.png$/i.test(value) &&
    !value.split('/').some((part) => !part || part === '.' || part === '..')
  );
}
/** Read only requested key. Never guess texture paths or accept arbitrary file/HTTP URIs. */
export function indexedIcon(value: unknown, classPath: string): ExporterIcon | null {
  const root = record(value);
  if (root.schemaVersion !== 1 || exportedClassReference(classPath) !== classPath) return null;
  const raw = record(record(root.icons)[classPath]);
  if (exportedClassReference(raw.texture) !== raw.texture || typeof raw.texture !== 'string')
    return null;
  if (!['self', 'descriptor', 'product', 'schematic'].includes(String(raw.source))) return null;
  for (const key of ['image', 'smallImage'])
    if (raw[key] !== undefined && !safeIconImage(raw[key])) return null;
  if (
    raw.smallTexture !== undefined &&
    (exportedClassReference(raw.smallTexture) !== raw.smallTexture ||
      raw.smallTexture === raw.texture)
  )
    return null;
  if (raw.smallImage !== undefined && raw.smallTexture === undefined) return null;
  for (const key of ['property', 'smallProperty'])
    if (raw[key] !== undefined && !propertyName(raw[key])) return null;
  if (raw.smallProperty !== undefined && raw.smallTexture === undefined) return null;
  const result: ExporterIcon = {
    texture: raw.texture,
    source: raw.source as ExporterIcon['source'],
  };
  for (const key of ['image', 'property', 'smallTexture', 'smallImage', 'smallProperty'] as const)
    if (typeof raw[key] === 'string') result[key] = raw[key];
  if (raw.via !== undefined) {
    const via = record(raw.via);
    if (
      exportedClassReference(via.class) !== via.class ||
      typeof via.class !== 'string' ||
      !propertyName(via.property)
    )
      return null;
    if (
      via.alternatives !== undefined &&
      (raw.source !== 'descriptor' ||
        !Number.isSafeInteger(via.alternatives) ||
        Number(via.alternatives) < 1 ||
        Number(via.alternatives) > 100_000)
    )
      return null;
    result.via = { class: via.class, property: via.property };
    if (via.alternatives !== undefined) result.via.alternatives = Number(via.alternatives);
  } else if (raw.source !== 'self') return null;
  return result;
}
export interface IndexedIconTexture {
  assetPath: string;
  image: string;
  width?: number;
  height?: number;
}
export function indexedTexture(
  value: unknown,
  assetPath: string,
  image: unknown,
): IndexedIconTexture | null {
  if (exportedClassReference(assetPath) !== assetPath || !safeIconImage(image)) return null;
  const raw = record(record(record(value).textures)[assetPath]);
  if (
    raw.status !== 'ok' ||
    raw.image !== image ||
    (raw.asset !== undefined && raw.asset !== assetPath)
  )
    return null;
  const texture: IndexedIconTexture = { assetPath, image };
  for (const key of ['width', 'height'] as const) {
    if (raw[key] !== undefined) {
      if (!Number.isInteger(raw[key]) || Number(raw[key]) < 1 || Number(raw[key]) > 4096)
        return null;
      texture[key] = Number(raw[key]);
    }
  }
  return texture;
}
export type RecipeIconHead =
  | { kind: 'other' }
  | { kind: 'unavailable' }
  | { kind: 'reference'; path: string; relation: 'first-output' | 'first-input' };
/** Recipe policy is stricter than index product[i]: always actual first output. */
export function recipeIconHead(value: unknown): RecipeIconHead {
  const raw = record(value);
  const ancestry = [
    raw.super,
    raw.parent,
    ...(Array.isArray(raw.superChain) ? raw.superChain : []),
  ];
  if (
    raw.path !== '/Script/FactoryGame.FGRecipe' &&
    !ancestry.includes('/Script/FactoryGame.FGRecipe')
  )
    return { kind: 'other' };
  const props = Array.isArray(raw.properties)
    ? raw.properties
    : Object.entries(record(raw.properties)).map(([name, value]) => ({ ...record(value), name }));
  const read = (name: string) => {
    const prop = props.map(record).find((prop) => prop.name === name);
    return prop && Object.prototype.hasOwnProperty.call(prop, 'default')
      ? firstRecipeItem(prop.default)
      : { state: 'unknown' as const };
  };
  const output = read('mProduct');
  if (output.state === 'first')
    return output.path
      ? { kind: 'reference', path: output.path, relation: 'first-output' }
      : { kind: 'unavailable' };
  if (output.state !== 'empty') return { kind: 'unavailable' };
  const input = read('mIngredients');
  return input.state === 'first' && input.path
    ? { kind: 'reference', path: input.path, relation: 'first-input' }
    : { kind: 'unavailable' };
}
