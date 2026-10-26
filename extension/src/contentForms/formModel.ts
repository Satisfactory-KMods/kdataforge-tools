import { isMap, isScalar, isSeq, parseAllDocuments, stringify } from 'yaml';

export type FormValue = string | number | boolean | null;
export interface FormField { path: (string | number)[]; label: string; type: 'string' | 'number' | 'integer' | 'boolean'; value: FormValue; required: boolean; enum?: FormValue[] }
export interface FormModel { uri: string; version: number; documentIndex: number; kind: string; fields: FormField[] }
export interface FormMessage { uri: string; version: number; documentIndex: number; path: (string | number)[]; operation: 'set'; value: FormValue }
export interface FormEdit { start: number; end: number; text: string; uri: string; version: number }

type Schema = { $id?: string; $defs?: Record<string, Schema>; type?: string; const?: unknown; enum?: unknown[];
  properties?: Record<string, Schema>; items?: Schema; required?: string[]; allOf?: Schema[]; anyOf?: Schema[];
  oneOf?: Schema[]; minLength?: number; maxLength?: number; pattern?: string;
  minimum?: number; maximum?: number; multipleOf?: number; $ref?: string };
const MAX_SOURCE = 4 * 1024 * 1024;
const MAX_FIELDS = 100;
const MAX_PATH = 12;
const has = (object: object, key: string): boolean => Object.hasOwn(object, key);

/** Caller supplies bundled schemas keyed by kind; $id resolves local catalog references only. */
function resolve(schema: Schema, catalog: Readonly<Record<string, Schema>>, depth = 0, context?: Schema): Schema | undefined {
  if (!schema.$ref) return schema;
  if (depth > 8) return undefined;
  const [id, pointer = ''] = schema.$ref.split('#');
  const candidates = Object.values(catalog).filter(item => item.$defs && has(item.$defs, pointer.split('/')[2] ?? ''));
  const owner = id ? Object.values(catalog).find(item => item.$id === id)
    : context ?? (candidates.length === 1 ? candidates[0] : undefined);
  if (!owner || !/^\/(?:\$defs|properties)\/[A-Za-z0-9_-]+$/.test(pointer)) return undefined;
  const [, group, name] = pointer.split('/');
  const target = group === '$defs' ? owner.$defs?.[name] : owner.properties?.[name];
  return target ? resolve(target, catalog, depth + 1, owner) : undefined;
}

function properties(schema: Schema, catalog: Readonly<Record<string, Schema>>): { fields: Record<string, Schema>; required: Set<string> } {
  const fields: Record<string, Schema> = Object.create(null);
  const required = new Set<string>();
  function visit(node: Schema, depth: number): void {
    if (depth > 8) return;
    const resolved = resolve(node, catalog);
    if (!resolved) return;
    for (const [name, definition] of Object.entries(resolved.properties ?? {})) fields[name] = definition;
    for (const name of resolved.required ?? []) required.add(name);
    for (const branch of [...resolved.allOf ?? [], ...resolved.anyOf ?? [], ...resolved.oneOf ?? []]) visit(branch, depth + 1);
  }
  visit(schema, 0);
  return { fields, required };
}

function scalarType(schema: Schema): FormField['type'] | undefined {
  return ['string', 'number', 'integer', 'boolean'].includes(schema.type ?? '') ? schema.type as FormField['type'] : undefined;
}

function documentAt(text: string, index: number) {
  if (Buffer.byteLength(text) > MAX_SOURCE || !Number.isSafeInteger(index) || index < 0 || index > 127)
    throw Error('Document out of bounds');
  const documents = parseAllDocuments(text, { uniqueKeys: true, strict: true });
  if (documents.length > 128 || documents.some(document => document.errors.length)) throw Error('Invalid YAML');
  const document = documents[index];
  if (!document || !isMap(document.contents)) throw Error('Unsupported YAML document');
  return document;
}

function kindOf(document: ReturnType<typeof documentAt>, uri: string): string {
  const kind = document.get('type');
  if (typeof kind === 'string') return kind;
  const file = uri.split(/[\\/]/).pop() ?? '';
  return /^pack\.ya?ml$/i.test(file) ? 'pack' : /^.+\.([a-z_]+)\.ya?ml$/i.exec(file)?.[1].toLowerCase() ?? '';
}

export function buildFormModel(text: string, uri: string, version: number, documentIndex: number,
  schemas: Readonly<Record<string, Schema>>): FormModel {
  const document = documentAt(text, documentIndex);
  const kind = kindOf(document, uri);
  if (!has(schemas, kind)) throw Error('Unsupported document kind');
  const { fields: declarations, required } = properties(schemas[kind], schemas);
  const fields: FormField[] = [];
  function visit(node: unknown, declaration: Schema, path: (string | number)[], required: boolean): void {
    if (fields.length >= MAX_FIELDS || path.length > MAX_PATH) return;
    const schema = resolve(declaration, schemas);
    if (!schema) return;
    const type = scalarType(schema);
    if (type && isScalar(node) && node.value !== undefined && node.value !== null) {
      const value = node.value;
      if (typeof value !== type && !(type === 'integer' && Number.isSafeInteger(value))) return;
      if (typeof value === 'string' && value.length > 4096) return;
      fields.push({ path, label: path.join('.'), type, value: value as FormValue, required,
        ...(schema.enum?.every(item => item === null || ['string', 'number', 'boolean'].includes(typeof item))
          ? { enum: schema.enum as FormValue[] } : {}) });
    } else if (isMap(node) && path.length < 4) {
      const nested = properties(schema, schemas);
      for (const [key, child] of Object.entries(nested.fields)) {
        if (!/^[A-Za-z_][A-Za-z0-9_-]{0,127}$/.test(key)) continue;
        visit(node.get(key, true), child, [...path, key], nested.required.has(key));
      }
    } else if (isSeq(node) && schema.items && path.length < 4) {
      for (let index = 0; index < Math.min(node.items.length, MAX_FIELDS); index++)
        visit(node.items[index], schema.items, [...path, index], false);
    }
  }
  for (const [key, schema] of Object.entries(declarations)) {
    if (fields.length >= MAX_FIELDS) break;
    visit(document.get(key, true), schema, [key], required.has(key));
  }
  return { uri, version, documentIndex, kind, fields };
}

export function validateFormMessage(input: unknown): FormMessage | null {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return null;
  const item = input as Record<string, unknown>;
  if (Object.keys(item).sort().join(',') !== 'documentIndex,operation,path,uri,value,version'
    || typeof item.uri !== 'string' || !item.uri.startsWith('file:') || item.uri.length > 4096
    || !Number.isSafeInteger(item.version) || (item.version as number) < 0
    || !Number.isSafeInteger(item.documentIndex) || (item.documentIndex as number) < 0 || (item.documentIndex as number) > 127
    || item.operation !== 'set' || !Array.isArray(item.path) || !item.path.length || item.path.length > MAX_PATH
    || !item.path.every(part => typeof part === 'string' && /^[A-Za-z_][A-Za-z0-9_-]{0,127}$/.test(part)
      || typeof part === 'number' && Number.isSafeInteger(part) && part >= 0 && part < MAX_FIELDS)
    || !(item.value === null || typeof item.value === 'boolean'
      || typeof item.value === 'number' && Number.isFinite(item.value)
      || typeof item.value === 'string' && item.value.length <= 4096 && !/[\u0000-\u001f]/.test(item.value))) return null;
  return item as unknown as FormMessage;
}

function validValue(value: FormValue, schema: Schema): boolean {
  const type = scalarType(schema);
  if (!type || schema.const !== undefined && value !== schema.const || schema.enum && !schema.enum.includes(value)) return false;
  if (type === 'string') {
    if (typeof value !== 'string' || value.length < (schema.minLength ?? 0) || value.length > (schema.maxLength ?? 4096)) return false;
    if (schema.pattern) {
      if (schema.pattern.length > 256) return false;
      try { if (!new RegExp(schema.pattern).test(value)) return false; } catch { return false; }
    }
  } else if (type === 'boolean') { if (typeof value !== 'boolean') return false; }
  else if (typeof value !== 'number' || type === 'integer' && !Number.isSafeInteger(value)
    || value < (schema.minimum ?? -Infinity) || value > (schema.maximum ?? Infinity)
    || schema.multipleOf !== undefined && value % schema.multipleOf !== 0) return false;
  return true;
}

/** Range replacement only. Host turns offsets into a version-checked WorkspaceEdit. */
export function planFormEdit(text: string, currentUri: string, currentVersion: number,
  schemas: Readonly<Record<string, Schema>>, input: unknown): FormEdit {
  const message = validateFormMessage(input);
  if (!message || message.uri !== currentUri || message.version !== currentVersion) throw Error('Invalid or stale form edit');
  const document = documentAt(text, message.documentIndex), kind = kindOf(document, currentUri);
  if (!has(schemas, kind)) throw Error('Unsupported form path');
  let schema: Schema | undefined = schemas[kind];
  for (const part of message.path) {
    schema = schema && resolve(schema, schemas);
    schema = schema && (typeof part === 'string' ? properties(schema, schemas).fields[part] : schema.items);
    if (!schema) throw Error('Unsupported form path');
  }
  schema = resolve(schema, schemas);
  const node = document.getIn(message.path, true);
  if (!schema || !isScalar(node) || !node.range || !validValue(message.value, schema)) throw Error('Unsupported form value');
  if (message.path[0] === 'type' || node.value === undefined || node.value === null) throw Error('Immutable or missing field');
  return { uri: currentUri, version: currentVersion, start: node.range[0], end: node.range[1],
    text: stringify(message.value, { lineWidth: 0 }).trimEnd() };
}
