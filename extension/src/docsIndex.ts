export const DOCS_ORIGIN = 'https://docs.k-mods.com';
export const DOCS_BASE = `${DOCS_ORIGIN}/kdataforge/`;
export const MAX_DOCS_PAGES = 400;
export const MAX_SECTION_TEXT = 6000;
const MAX_INDEX_BYTES = 16 * 1024 * 1024;

export interface DocsSection { anchor: string; heading: string; level: number; text: string }
export interface DocsPage { path: string; title: string; description: string; sections: DocsSection[] }
export interface FieldDoc { ref: string; values?: string }
export type FieldEntries = Record<string, FieldDoc>;
export interface DocsFieldMap {
  kinds: Record<string, FieldEntries>;
  common: FieldEntries;
  values: Record<string, FieldEntries>;
  prefixes: FieldEntries;
  kindDefaults?: Record<string, string>;
}
export interface DocsIndex {
  version: 1;
  origin: string;
  generatedAt: string;
  contentTag: string;
  pages: DocsPage[];
  kindPages: Record<string, string>;
  fields: DocsFieldMap;
}

export function referencedDocs(fields: DocsFieldMap): Map<string, Set<string>> {
  const references = new Map<string, Set<string>>();
  const entries = [...Object.values(fields.kinds).flatMap(Object.values), ...Object.values(fields.common),
    ...Object.values(fields.values).flatMap(Object.values), ...Object.values(fields.prefixes)];
  const refs = [...entries.map(entry => entry.ref), ...Object.values(fields.kindDefaults ?? {})];
  for (const ref of refs) {
    const [path, anchor = ''] = docsRef(ref).split('#');
    const anchors = references.get(path) ?? new Set<string>();
    anchors.add(anchor);
    references.set(path, anchors);
  }
  return references;
}

export function hasDocsAnchors(page: DocsPage, anchors: ReadonlySet<string>): boolean {
  const available = new Set(page.sections.map(section => section.anchor));
  return [...anchors].every(anchor => available.has(anchor));
}

/** Accept catalog references or canonical external URLs; reject normalization tricks. */
export function docsUrl(input: unknown): string {
  if (typeof input !== 'string' || input.length > 1024) throw new Error('Invalid docs reference');
  let ref = input;
  if (ref.startsWith(DOCS_BASE)) ref = ref.slice(DOCS_BASE.length).replace(/\/(?=#|$)/, '');
  if (!/^(?:[a-z0-9-]+(?:\/[a-z0-9-]+)*)?(?:#[\w-]+)?$/.test(ref)) {
    throw new Error('Invalid docs reference');
  }
  const [path, anchor] = ref.split('#');
  return `${DOCS_BASE}${path ? `${path}/` : ''}${anchor ? `#${anchor}` : ''}`;
}

export function docsRef(input: string): string {
  return docsUrl(input).slice(DOCS_BASE.length).replace(/\/(?=#|$)/, '');
}

export function decodeEntities(text: string): string {
  const entities: Record<string, string> = {
    amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', ndash: '–', mdash: '—',
    hellip: '…', lsquo: '‘', rsquo: '’', ldquo: '“', rdquo: '”', copy: '©',
  };
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, entity: string) => {
    if (!entity.startsWith('#')) return entities[entity.toLowerCase()] ?? whole;
    const code = entity[1].toLowerCase() === 'x' ? parseInt(entity.slice(2), 16) : Number(entity.slice(1));
    return code > 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff)
      ? String.fromCodePoint(code) : '�';
  });
}

function plainText(html: string): string {
  return decodeEntities(html.replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<(script|style|svg|button|template|noscript)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, '')
    .replace(/<[^>]*>/g, ' ')).replace(/[\u0000-\u001f\u007f]/g, ' ').replace(/\s+/g, ' ').trim();
}

/** Starlight page structure verified at docs.k-mods.com/kdataforge/features/operations/. */
export function parseDocsPage(html: string, url: string): DocsPage {
  const path = docsRef(url);
  if (path.includes('#')) throw new Error('Expected docs page URL');
  const title = plainText(html.match(/<h1\b[^>]*>[\s\S]*?<\/h1>/i)?.[0] ?? '');
  const marker = /<div\b[^>]*class=["'][^"']*\bsl-markdown-content\b[^"']*["'][^>]*>/i.exec(html);
  if (!title || !marker) throw new Error('Invalid docs page');
  let body = html.slice(marker.index + marker[0].length).split(/<footer\b/i)[0];
  body = body.replace(/<(script|style|svg|button|template|noscript)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, '')
    .replace(/<a\b[^>]*class=["'][^"']*\bsl-anchor-link\b[^"']*["'][^>]*>[\s\S]*?<\/a>/gi, '');
  const headings = [...body.matchAll(/<h([2-6])\b[^>]*\bid=["']([\w-]+)["'][^>]*>([\s\S]*?)<\/h\1>/gi)];
  const sections: DocsSection[] = [{ anchor: '', heading: title, level: 1,
    text: plainText(body.slice(0, headings[0]?.index)).slice(0, MAX_SECTION_TEXT) }];
  for (let i = 0; i < headings.length; i++) {
    const match = headings[i];
    sections.push({ anchor: match[2], heading: plainText(match[3]), level: Number(match[1]),
      text: plainText(body.slice(match.index! + match[0].length, headings[i + 1]?.index)).slice(0, MAX_SECTION_TEXT) });
  }
  const meta = html.match(/<meta\b[^>]*name=["']description["'][^>]*>/i)?.[0] ?? '';
  const description = /\bcontent=(["'])([\s\S]*?)\1/i.exec(meta)?.[2] ?? '';
  return { path, title: title.slice(0, 500), description: plainText(description).slice(0, 2000), sections };
}

function record(input: unknown): Record<string, unknown> {
  if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error('Expected object');
  return input as Record<string, unknown>;
}

export function normalizeFieldPath(path: string | readonly string[]): string {
  return (typeof path === 'string' ? path : path.join('.')).replace(/^\$(?:\.|(?=\[|$))/, '').replace(/\[(?:\d*)\]/g, '');
}

function catalogEntry(input: unknown, inherited?: string): FieldDoc | undefined {
  const entry = record(input);
  const ref = typeof entry.docs === 'string' ? docsRef(entry.docs) : inherited;
  if (ref === undefined) return undefined;
  return { ref, ...(typeof entry.values === 'string' ? { values: entry.values } : {}) };
}

/** Flatten catalog uses, preserving explicit kind entries over inherited definition entries. */
export function buildFieldMap(input: unknown): DocsFieldMap {
  const catalog = record(input);
  const defs = record(catalog.defs ?? {});
  function expand(inputTable: unknown, inherited?: string, depth = 0): FieldEntries {
    const table = record(inputTable);
    const output: FieldEntries = Object.create(null);
    const usesEntries = Object.entries(table).sort(([left], [right]) =>
      normalizeFieldPath(right).split('.').length - normalizeFieldPath(left).split('.').length);
    for (const [path, value] of usesEntries) {
      const entry = catalogEntry(value, inherited);
      if (entry) output[normalizeFieldPath(path)] = entry;
    }
    if (depth >= 5) return output;
    for (const [path, value] of usesEntries) {
      const entry = record(value);
      const uses = typeof entry.uses === 'string' ? [entry.uses] : entry.uses;
      if (!Array.isArray(uses)) continue;
      const ref = catalogEntry(value, inherited)?.ref;
      for (const name of uses) {
        if (typeof name !== 'string' || !Object.hasOwn(defs, name)) continue;
        for (const [child, doc] of Object.entries(expand(defs[name], ref, depth + 1))) {
          output[`${normalizeFieldPath(path)}.${child}`] ??= doc;
        }
      }
    }
    return output;
  }
  const flattenGroups = (input: unknown): Record<string, FieldEntries> => Object.fromEntries(
    Object.entries(record(input ?? {})).map(([group, table]) => [group, expand(table)]));
  const kindDefaults = Object.fromEntries(Object.entries(record(catalog.kinds ?? {})).flatMap(([kind, table]) => {
    const refs = Object.values(record(table)).flatMap(value => {
      const doc = catalogEntry(value);
      return doc ? [doc.ref.split('#')[0]] : [];
    });
    const best = mostFrequentPage(refs);
    return best === undefined ? [] : [[kind, best]];
  }));
  return { kinds: flattenGroups(catalog.kinds), common: expand(catalog.common ?? {}),
    values: flattenGroups(catalog.values), prefixes: expand(catalog.prefixes ?? {}), kindDefaults };
}

function matchingEntry(entries: FieldEntries, path: string): FieldDoc | undefined {
  if (Object.hasOwn(entries, path)) return entries[path];
  const parts = path.split('.');
  const candidates = Object.entries(entries).filter(([pattern]) => {
    const patternParts = pattern.split('.');
    return patternParts.length === parts.length && patternParts.every((part, i) => part === '*' || part === parts[i]);
  });
  candidates.sort(([left], [right]) => left.split('*').length - right.split('*').length || left.localeCompare(right));
  return candidates[0]?.[1];
}

export function lookupField(map: DocsFieldMap, kind: string, input: string | readonly string[]): FieldDoc | undefined {
  const path = normalizeFieldPath(input);
  const entries = map.kinds[kind === 'mam' ? 'research' : kind] ?? {};
  const exact = matchingEntry(entries, path);
  if (exact) return exact;
  const parts = path.split('.');
  const conditionAt = parts.findIndex(part => ['conditions', 'conditionBehavior', 'conditionBehaivor'].includes(part));
  if (conditionAt >= 0) {
    const conditionParts = parts.slice(conditionAt);
    while (conditionParts.length) {
      const condition = matchingEntry(map.common, conditionParts.join('.'));
      if (condition) return condition;
      conditionParts.pop();
    }
  }
  const common = matchingEntry(map.common, path);
  if (common) return common;
  while (parts.length > 1) {
    parts.pop();
    const parent = matchingEntry(entries, parts.join('.')) ?? matchingEntry(map.common, parts.join('.'));
    if (parent) return parent;
  }
  return undefined;
}

function mostFrequentPage(refs: readonly string[]): string | undefined {
  const counts = new Map<string, number>();
  for (const ref of refs) counts.set(ref, (counts.get(ref) ?? 0) + 1);
  return [...counts].sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))[0]?.[0];
}

export function deriveKindPages(pages: readonly DocsPage[], fields: DocsFieldMap): Record<string, string> {
  const result: Record<string, string> = Object.create(null);
  for (const page of pages) {
    const kind = /type:\s*([a-z_]+)/i.exec(page.title)?.[1].toLowerCase();
    if (kind) result[kind] = page.path;
  }
  const paths = new Set(pages.map(page => page.path));
  for (const [kind, entries] of Object.entries(fields.kinds)) {
    if (Object.hasOwn(result, kind)) continue;
    const typePage = fields.values.type?.[kind]?.ref.split('#')[0];
    if (typePage !== undefined && paths.has(typePage)) {
      result[kind] = typePage;
      continue;
    }
    const catalogPage = fields.kindDefaults && Object.hasOwn(fields.kindDefaults, kind) ? fields.kindDefaults[kind] : undefined;
    const best = catalogPage !== undefined && paths.has(catalogPage) ? catalogPage
      : mostFrequentPage(Object.values(entries).map(doc => doc.ref.split('#')[0]).filter(path => paths.has(path)));
    if (best !== undefined) result[kind] = best;
  }
  if (result.research) result.mam = result.research;
  return result;
}

const STOPWORDS = new Set(['the', 'and', 'for', 'with', 'this', 'that', 'from', 'are', 'you', 'your', 'can']);
export function docsTokens(text: string): string[] {
  const words = `${text} ${text.replace(/([a-z0-9])([A-Z])/g, '$1 $2').replace(/_/g, ' ')}`.toLowerCase().match(/[a-z0-9_]+/g) ?? [];
  return words.filter(word => word.length >= 3 && !STOPWORDS.has(word));
}

export interface DocsSearchHit { ref: string; score: number }
export class DocsSearch {
  private readonly documents: { ref: string; counts: Map<string, number>; length: number }[];
  private readonly frequencies = new Map<string, number>();
  private readonly averageLength: number;

  constructor(index: DocsIndex) {
    this.documents = index.pages.flatMap(page => page.sections.map(section => {
      const words = docsTokens(`${section.text} ${section.heading} ${section.heading} ${section.heading} ${page.title}`);
      const counts = new Map<string, number>();
      for (const word of words) counts.set(word, (counts.get(word) ?? 0) + 1);
      for (const word of counts.keys()) this.frequencies.set(word, (this.frequencies.get(word) ?? 0) + 1);
      return { ref: `${page.path}${section.anchor ? `#${section.anchor}` : ''}`, counts, length: words.length };
    }));
    this.averageLength = this.documents.reduce((sum, doc) => sum + doc.length, 0) / this.documents.length || 1;
  }

  search(query: string, limit = 5): DocsSearchHit[] {
    const words = new Set(docsTokens(query));
    return this.documents.map(doc => {
      let score = 0;
      for (const word of words) {
        const count = doc.counts.get(word) ?? 0;
        if (!count) continue;
        const frequency = this.frequencies.get(word) ?? 0;
        const inverse = Math.log(1 + (this.documents.length - frequency + 0.5) / (frequency + 0.5));
        score += inverse * count * 2.2 / (count + 1.2 * (0.25 + 0.75 * doc.length / this.averageLength));
      }
      return { ref: doc.ref, score };
    }).filter(hit => hit.score > 0).sort((left, right) => right.score - left.score || left.ref.localeCompare(right.ref))
      .slice(0, Math.max(0, Math.min(100, limit)));
  }
}

function boundedString(value: unknown, max: number): string {
  if (typeof value !== 'string' || value.length > max || /[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(value)) {
    throw new Error('Invalid docs string');
  }
  return value;
}

function readEntries(input: unknown): FieldEntries {
  const entries = Object.entries(record(input));
  if (entries.length > 20000) throw new Error('Too many docs fields');
  return Object.fromEntries(entries.map(([key, value]) => {
    boundedString(key, 500);
    const entry = record(value);
    const ref = docsRef(boundedString(entry.ref, 1024));
    return [key, { ref, ...(entry.values === undefined ? {} : { values: boundedString(entry.values, 100) }) }];
  }));
}

/** Untrusted online/cache data is copied into a validated, bounded domain object. */
export function readDocsIndex(input: unknown): DocsIndex {
  if (typeof input === 'string' && Buffer.byteLength(input) > MAX_INDEX_BYTES) throw new Error('Docs index too large');
  const index = record(typeof input === 'string' ? JSON.parse(input) : input);
  if (index.version !== 1 || index.origin !== DOCS_ORIGIN || !Array.isArray(index.pages)
    || !index.pages.length || index.pages.length > MAX_DOCS_PAGES) throw new Error('Invalid docs index');
  const generatedAt = boundedString(index.generatedAt, 100);
  if (!Number.isFinite(Date.parse(generatedAt))) throw new Error('Invalid docs timestamp');
  const pages = index.pages.map(inputPage => {
    const page = record(inputPage);
    const path = docsRef(boundedString(page.path, 500));
    if (path.includes('#') || !Array.isArray(page.sections) || !page.sections.length || page.sections.length > 300) {
      throw new Error('Invalid docs page');
    }
    const sections = page.sections.map(inputSection => {
      const section = record(inputSection);
      const anchor = boundedString(section.anchor, 500);
      if ((anchor && !/^[\w-]+$/.test(anchor)) || !Number.isInteger(section.level)
        || Number(section.level) < 1 || Number(section.level) > 6) throw new Error('Invalid docs section');
      return { anchor, heading: boundedString(section.heading, 500), level: Number(section.level),
        text: boundedString(section.text, MAX_SECTION_TEXT) };
    });
    if (sections[0].anchor !== '' || new Set(sections.map(section => section.anchor)).size !== sections.length) {
      throw new Error('Invalid docs anchors');
    }
    return { path, title: boundedString(page.title, 500), description: boundedString(page.description, 2000), sections };
  });
  const paths = new Set(pages.map(page => page.path));
  if (paths.size !== pages.length) throw new Error('Duplicate docs page');
  const sourceFields = record(index.fields);
  const readGroups = (inputGroups: unknown) => Object.fromEntries(Object.entries(record(inputGroups)).map(
    ([name, entries]) => [boundedString(name, 100), readEntries(entries)]));
  const fields: DocsFieldMap = { kinds: readGroups(sourceFields.kinds), common: readEntries(sourceFields.common),
    values: readGroups(sourceFields.values), prefixes: readEntries(sourceFields.prefixes) };
  if (sourceFields.kindDefaults !== undefined) {
    fields.kindDefaults = Object.fromEntries(Object.entries(record(sourceFields.kindDefaults)).map(([kind, ref]) =>
      [boundedString(kind, 100), docsRef(boundedString(ref, 500))]));
  }
  const kindPages = Object.fromEntries(Object.entries(record(index.kindPages)).map(([kind, ref]) => {
    const path = docsRef(boundedString(ref, 500));
    if (!paths.has(path)) throw new Error('Unknown kind page');
    return [boundedString(kind, 100), path];
  }));
  return { version: 1, origin: DOCS_ORIGIN, generatedAt, contentTag: boundedString(index.contentTag, 1024),
    pages, fields, kindPages };
}
