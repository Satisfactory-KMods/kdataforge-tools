import { DocumentSnapshot } from './scriptAnalysis/graph';

/** Provider-neutral UTF-16 edit. Hosts must revalidate the whole plan immediately before applying it. */
export interface ContentTextEdit {
  readonly fileName: string;
  readonly start: number;
  readonly end: number;
  readonly oldText: string;
  readonly newText: string;
  readonly version: string | number;
}
export interface ContentEditPlan {
  readonly safe: boolean;
  readonly generation: number;
  readonly edits: readonly ContentTextEdit[];
  /** Includes analyzed documents without edits: changing one may introduce a dynamic reference. */
  readonly documents: readonly { fileName: string; version: string | number }[];
  readonly blockers: readonly string[];
}

export function createContentEditPlan(generation: number, documents: readonly DocumentSnapshot[],
  edits: readonly ContentTextEdit[], blockers: readonly string[] = []): ContentEditPlan {
  return combineContentRenamePlans([{ generation, safe: !blockers.length, edits,
    documents: documents.map(({ fileName, version }) => ({ fileName, version })), blockers }], documents, generation);
}

/** Combine TS and YAML/native plans into one transaction; never apply a partially safe plan. */
export function combineContentRenamePlans(plans: readonly ContentEditPlan[], currentDocuments: readonly DocumentSnapshot[],
  currentGeneration: number): ContentEditPlan {
  const blockers: string[] = [];
  const snapshots = new Map<string, DocumentSnapshot>();
  for (const document of currentDocuments) {
    if (snapshots.has(document.fileName)) blockers.push(`duplicate-snapshot:${document.fileName}`);
    snapshots.set(document.fileName, document);
  }
  const documents = new Map<string, string | number>();
  const edits = new Map<string, ContentTextEdit>();
  if (!plans.length) blockers.push('missing-edit-plan');
  for (const plan of plans) {
    blockers.push(...plan.blockers);
    if (!plan.safe) blockers.push('unsafe-provider-plan');
    if (plan.generation !== currentGeneration) blockers.push('stale-generation');
    for (const document of plan.documents) {
      if (documents.has(document.fileName) && documents.get(document.fileName) !== document.version)
        blockers.push(`conflicting-version:${document.fileName}`);
      documents.set(document.fileName, document.version);
      if (snapshots.get(document.fileName)?.version !== document.version) blockers.push(`stale-document:${document.fileName}`);
    }
    for (const edit of plan.edits) {
      const snapshot = snapshots.get(edit.fileName);
      if (!plan.documents.some(document => document.fileName === edit.fileName && document.version === edit.version)
        || !snapshot || snapshot.version !== edit.version) { blockers.push(`stale-edit:${edit.fileName}`); continue; }
      if (!Number.isSafeInteger(edit.start) || !Number.isSafeInteger(edit.end) || edit.start < 0
        || edit.end <= edit.start || edit.end > snapshot.text.length || typeof edit.newText !== 'string'
        || snapshot.text.slice(edit.start, edit.end) !== edit.oldText) { blockers.push(`text-mismatch:${edit.fileName}`); continue; }
      const key = JSON.stringify([edit.fileName, edit.start, edit.end]);
      const existing = edits.get(key);
      if (existing && existing.newText !== edit.newText) blockers.push(`conflicting-edit:${edit.fileName}`);
      edits.set(key, edit);
    }
  }
  const sorted = [...edits.values()].sort((left, right) => left.fileName.localeCompare(right.fileName) || right.start - left.start);
  for (let index = 1; index < sorted.length; index++) {
    const prior = sorted[index - 1], next = sorted[index];
    if (prior.fileName === next.fileName && next.end > prior.start) blockers.push(`overlapping-edits:${next.fileName}`);
  }
  const unique = [...new Set(blockers)];
  return { safe: !unique.length, generation: currentGeneration, edits: unique.length ? [] : sorted,
    documents: [...documents].map(([fileName, version]) => ({ fileName, version })), blockers: unique };
}

export function validateContentEditPlan(plan: ContentEditPlan, currentDocuments: readonly DocumentSnapshot[],
  currentGeneration: number): ContentEditPlan {
  return combineContentRenamePlans([plan], currentDocuments, currentGeneration);
}
