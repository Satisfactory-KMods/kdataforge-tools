import type { StructuredDiagnostic } from './diagnosticExport';

export interface DiagnosticEvidence {
  kind: string;
  detail: string;
  source?: string;
}

export interface ExplainedDiagnostic {
  title: string;
  location?: string;
  explanation: string;
  evidence: DiagnosticEvidence[];
  knownCause: boolean;
}

/** Show supplied evidence verbatim as plain text. A diagnostic message alone is not a cause. */
export function explainDiagnostic(diagnostic: StructuredDiagnostic & { evidence?: readonly DiagnosticEvidence[] }): ExplainedDiagnostic {
  const evidence = (diagnostic.evidence ?? []).filter(item => typeof item.kind === 'string'
    && typeof item.detail === 'string' && item.kind.length <= 100 && item.detail.length <= 4000)
    .slice(0, 32).map(item => ({ kind: item.kind, detail: item.detail,
      ...(typeof item.source === 'string' ? { source: item.source } : {}) }));
  const cause = evidence.find(item => item.kind === 'cause');
  return {
    title: `${diagnostic.code}: ${diagnostic.message}`,
    ...(diagnostic.file ? { location: `${diagnostic.file}${diagnostic.yamlPath ? ` ${diagnostic.yamlPath}` : ''}` } : {}),
    explanation: cause?.detail ?? 'Cause unknown: no structured cause evidence supplied.',
    evidence,
    knownCause: cause !== undefined,
  };
}
