import { safeIconImage } from './exporterIcons';

export type TextPreview =
  | { state: 'resolved'; text: string; origin: 'literal' | 'localization' | 'source' }
  | { state: 'unresolved'; text: string; reason: string };

export interface LocalizedSource {
  namespace: string;
  culture?: string;
  entries: Record<string, string | { text: string; source?: string }>;
}

/** Local, parsed localization documents only. YAML uses ns; script values use namespace. */
export function previewText(value: unknown, sources: readonly LocalizedSource[], culture?: string): TextPreview {
  if (typeof value === 'string') return { state: 'resolved', text: value, origin: 'literal' };
  if (!value || typeof value !== 'object' || Array.isArray(value))
    return { state: 'unresolved', text: '', reason: 'Unsupported text value' };
  const input = value as Record<string, unknown>;
  if (typeof input.stringTable === 'string')
    return { state: 'unresolved', text: `[${input.stringTable}:${String(input.key ?? '')}]`, reason: 'String table unavailable locally' };
  const namespace = typeof input.ns === 'string' ? input.ns : input.namespace;
  const key = input.key;
  if (typeof namespace !== 'string' || typeof key !== 'string' || !key) {
    if (typeof input.source === 'string') return { state: 'resolved', text: input.source, origin: 'source' };
    return { state: 'unresolved', text: '', reason: 'Missing namespace or key' };
  }
  for (const source of sources) {
    if (source.namespace !== namespace || source.culture !== culture
      || !Object.hasOwn(source.entries, key)) continue;
    const entry = source.entries[key];
    const text = typeof entry === 'string' ? entry : entry?.text;
    if (typeof text === 'string') return { state: 'resolved', text, origin: 'localization' };
  }
  if (typeof input.source === 'string') return { state: 'resolved', text: input.source, origin: 'source' };
  return { state: 'unresolved', text: `[${namespace}:${key}]`, reason: 'No matching local localization entry or source' };
}

export const MAX_PREVIEW_PNG_BYTES = 4 * 1024 * 1024;
export const MAX_PREVIEW_ARCHIVE_BYTES = 2 * 1024 * 1024 * 1024;

const crcTable = Uint32Array.from({ length: 256 }, (_, index) => {
  let value = index;
  for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ (value & 1 ? 0xedb88320 : 0);
  return value >>> 0;
});

function crc32(bytes: Buffer): number {
  let value = 0xffffffff;
  for (const byte of bytes) value = (value >>> 8) ^ crcTable[(value ^ byte) & 255];
  return (value ^ 0xffffffff) >>> 0;
}

function validPng(bytes: Buffer): boolean {
  if (bytes.length < 57 || !bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return false;
  let offset = 8, chunks = 0, hasImage = false, ended = false;
  while (offset + 12 <= bytes.length && chunks++ < 8192) {
    const size = bytes.readUInt32BE(offset), type = bytes.toString('ascii', offset + 4, offset + 8);
    if (size > MAX_PREVIEW_PNG_BYTES || offset + size + 12 > bytes.length
      || crc32(bytes.subarray(offset + 4, offset + size + 8)) !== bytes.readUInt32BE(offset + size + 8)) return false;
    if (chunks === 1 && (type !== 'IHDR' || size !== 13 || !bytes.readUInt32BE(offset + 8)
      || !bytes.readUInt32BE(offset + 12) || bytes.readUInt32BE(offset + 8) > 4096
      || bytes.readUInt32BE(offset + 12) > 4096)) return false;
    if (type === 'IDAT') hasImage = true;
    offset += size + 12;
    if (type === 'IEND') { ended = size === 0; break; }
  }
  return ended && hasImage && offset === bytes.length;
}

export interface VerifiedLocalIcon {
  archiveBytes: number;
  entryPath: string;
  dataUrl: string;
  /** True only after host verified archive identity and entry through native archive reader. */
  verified: boolean;
}

/** Accept native-verified local archive image only; never turn user paths into remote requests. */
export function previewIcon(icon: VerifiedLocalIcon): { dataUrl: string } | null {
  if (!icon.verified || !Number.isSafeInteger(icon.archiveBytes) || icon.archiveBytes < 1
    || icon.archiveBytes > MAX_PREVIEW_ARCHIVE_BYTES || !safeIconImage(icon.entryPath)
    || !/^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(icon.dataUrl)
    || icon.dataUrl.length > Math.ceil(MAX_PREVIEW_PNG_BYTES / 3) * 4 + 32) return null;
  const encoded = icon.dataUrl.slice('data:image/png;base64,'.length);
  const bytes = Buffer.from(encoded, 'base64');
  if (bytes.length > MAX_PREVIEW_PNG_BYTES || bytes.toString('base64') !== encoded || !validPng(bytes)) return null;
  return { dataUrl: icon.dataUrl };
}
