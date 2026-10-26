/** Portable structured diagnostics from validator/LSP. No process execution or PATH lookup. */
export interface StructuredDiagnostic {
  severity: 'error' | 'warning' | 'information' | 'hint' | number;
  advisory?: 'information' | 'hint' | null;
  code: string;
  message: string;
  file?: string | null;
  document?: number | null;
  yamlPath?: string;
  line?: number | null;
  column?: number | null;
  source?: string;
  schema?: string | null;
  range?: { start: { line: number; character: number }; end: { line: number; character: number } };
}

function level(severity: StructuredDiagnostic['severity']): 'error' | 'warning' | 'note' {
  if (severity === 'warning' || severity === 2) return 'warning';
  if (severity === 'information' || severity === 'hint' || severity === 3 || severity === 4) return 'note';
  return 'error';
}

function displayLevel(item: StructuredDiagnostic): 'error' | 'warning' | 'note' {
  return item.advisory ? 'note' : level(item.severity);
}

function location(diagnostic: StructuredDiagnostic): { uri: string; line: number; column: number } | undefined {
  if (!diagnostic.file) return undefined;
  const line = diagnostic.range ? diagnostic.range.start.line + 1 : diagnostic.line ?? 1;
  const column = diagnostic.range ? diagnostic.range.start.character + 1 : diagnostic.column ?? 1;
  return { uri: diagnostic.file.replace(/\\/g, '/'), line: Math.max(1, line), column: Math.max(1, column) };
}

function ordered(diagnostics: readonly StructuredDiagnostic[]): StructuredDiagnostic[] {
  return [...diagnostics].sort((a, b) => {
    const left = location(a), right = location(b);
    const keys = [left?.uri ?? '', String(left?.line ?? 0).padStart(10, '0'),
      String(left?.column ?? 0).padStart(10, '0'), a.code, a.message, a.yamlPath ?? '', String(a.document ?? '')];
    const other = [right?.uri ?? '', String(right?.line ?? 0).padStart(10, '0'),
      String(right?.column ?? 0).padStart(10, '0'), b.code, b.message, b.yamlPath ?? '', String(b.document ?? '')];
    for (let i = 0; i < keys.length; i++) if (keys[i] !== other[i]) return keys[i] < other[i] ? -1 : 1;
    return JSON.stringify([displayLevel(a), a.severity, a.source, a.schema, a.advisory, a.range])
      .localeCompare(JSON.stringify([displayLevel(b), b.severity, b.source, b.schema, b.advisory, b.range]));
  });
}

/** Stable JSON array; retains supplied structured fields, never adds timestamps. */
export function exportDiagnosticsJson(diagnostics: readonly StructuredDiagnostic[]): string {
  return JSON.stringify(ordered(diagnostics).map(item => ({
    severity: item.severity, code: item.code, message: item.message,
    ...(item.file === undefined ? {} : { file: item.file }),
    ...(item.document === undefined ? {} : { document: item.document }),
    ...(item.yamlPath === undefined ? {} : { yamlPath: item.yamlPath }),
    ...(item.line === undefined ? {} : { line: item.line }),
    ...(item.column === undefined ? {} : { column: item.column }),
    ...(item.source === undefined ? {} : { source: item.source }),
    ...(item.schema === undefined ? {} : { schema: item.schema }),
    ...(item.advisory === undefined ? {} : { advisory: item.advisory }),
    ...(item.range === undefined ? {} : { range: item.range }),
  })), null, 2) + '\n';
}

export function exportDiagnosticsSarif(diagnostics: readonly StructuredDiagnostic[]): string {
  const sorted = ordered(diagnostics);
  const rules = [...new Set(sorted.map(item => item.code))].sort().map(id => ({ id }));
  const results = sorted.map(item => {
    const where = location(item);
    return {
      ruleId: item.code,
      level: displayLevel(item),
      message: { text: item.message },
      ...(where ? { locations: [{ physicalLocation: { artifactLocation: { uri: where.uri },
        region: { startLine: where.line, startColumn: where.column } } }] } : {}),
      properties: {
        ...(item.document === undefined || item.document === null ? {} : { document: item.document }),
        ...(item.yamlPath ? { yamlPath: item.yamlPath } : {}),
        ...(item.schema ? { schema: item.schema } : {}),
        ...(item.source ? { source: item.source } : {}),
      },
    };
  });
  return JSON.stringify({ version: '2.1.0', $schema: 'https://json.schemastore.org/sarif-2.1.0.json',
    runs: [{ tool: { driver: { name: 'KDataForge', rules } }, results }] }, null, 2) + '\n';
}
