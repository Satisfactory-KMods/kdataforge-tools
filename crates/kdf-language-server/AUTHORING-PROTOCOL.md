# KDataForge authoring protocol v1

All methods below are read-only JSON-RPC requests over the existing LSP stdio connection.
No request writes files or applies edits. Existing `kdf/classes`, `kdf/classInfo`,
`kdf/classReferenceSlots`, `kdf/affectedElements`, `kdf/inferPropertyOwner`, `kdf/operations`,
`kdf/previewBinding`, `kdf/revalidate`, and `kdf/reindex` retain their request/response shapes.
Reflection results gain an additive `digest` (SHA-256 of the loaded archive, or null).

The initialize result advertises:

```json
{"capabilities":{"experimental":{"kdfAuthoring":{"schemaVersion":1,"completionLimit":200,"snapshotGuards":true,"renamePlans":true}}}}
```

## Shared contracts

Positions/ranges use zero-based lines and UTF-16 character offsets, end exclusive.
`document` is the zero-based YAML document index within a file. Paths are canonical absolute
filesystem paths; LSP edit URIs retain the spelling used by the client for open documents.
Digests are lowercase, 64-character SHA-256 hex. Document digests hash the decoded document
text encoded as UTF-8, without changing line endings; they do not hash BOM/UTF-16 storage bytes.

```ts
type SnapshotToken = {
  schemaVersion: 1;
  generation: number; // opaque server generation; compare, never manufacture
  digest: string; // inventory, document versions/text digests, reflection digest, completeness
  reflectionDigest: string | null;
};
type ContentIdentity = {
  pack: string; // runtime-normalized pack ref: punctuation -> underscore, lowercase
  namespace: "class" | "asset" | "eventRule" | "resourcePlacement";
  id: string;
};
type DocumentSnapshot = {
  path: string;
  version: number | null; // open LSP version, null for disk-only files
  digest: string;
};
type ContentSymbol = {
  identity: ContentIdentity;
  packRef: string; id: string; kind: string; path: string;
  document: number; range: Range;
  generatedPath: string | null; parent: string | null;
};
```

Class/asset IDs fold case and map punctuation to `_`; assets, data assets, curves and sounds
share the asset namespace. Event rule IDs fold case while preserving punctuation. Resource
placement IDs preserve case/punctuation. Config IDs and pack refs are not content declarations.

Optional guards on read requests:

```ts
type Guards = {
  snapshot?: SnapshotToken;
  textDocument?: { uri: string; version?: number | null };
};
```

A supplied snapshot must match exactly. A supplied document/version must belong to the snapshot
and match its current version. `-32801` means content modified/stale: discard the answer/plan,
refresh, then retry the current request. `-32602` means invalid input or a semantic refusal.
Standard `$/cancelRequest` cancellation is handled by the LSP transport (`-32800`). Newer
buffer/config generations suppress in-flight results. A supplied `requestGeneration` provides
an additional monotonic, connection-wide completion fence; reset it only after restarting LSP.

## Snapshot and symbols

`kdf/exportSnapshot(Guards | {})` forces a current disk/buffer inventory and verifies the bound
reflection archive's bytes, including same-size/same-mtime replacements. Response:

```ts
{
  snapshot: SnapshotToken; root: string;
  complete: boolean; // authored YAML inventory, independent of reflection
  reflectionComplete: boolean;
  limitations: string[]; renameBlockers: string[];
  documents: DocumentSnapshot[];
  symbols: ContentSymbol[];
  references: Array<{
    path: string; document: number; range: Range | null; value: string;
    candidates: ContentIdentity[]; typed: boolean;
  }>;
}
```

No raw document text is exported. Inventory limits: 10,000 YAML files, 64 MiB total decoded
text, 8 MiB/file, 1,024 YAML documents/file. Truncation/read/parse failures produce explicit
incompleteness. Runtime-ignored directories (dot directories, Docs, Scripts) are excluded.
Declared scripts, includes, redirects, aliases and dynamic references block automatic rename.
The model covers the selected workspace only; references in external packs are not surveyed.

`kdf/contentSymbols(Guards & {limit?: number})` returns
`{schemaVersion:1,snapshot,items:ContentSymbol[],complete,isIncomplete}`.
The default/max limit is 200. `textDocument` restricts symbols to that file. Standard
`textDocument/documentSymbol` returns the same declarations as LSP document symbols.

## Q15: bounded completion

`kdf/contentCompletion` parameters:

```ts
Guards & {
  kind: "content" | "classes" | "assets";
  query?: string; // case-insensitive substring in label/path, max 1,024 UTF-8 bytes
  limit?: number; // default 200, clamped to 1..200
  pack?: string; // content: pack ref filter, normalized by server
  namespace?: ContentIdentity["namespace"]; // content only
  baseClass?: string; // classes: inheritance filter; assets: required indexed class
  requestGeneration?: number; // monotonically increasing across this connection
}
```

Response:

```ts
{
  schemaVersion: 1; snapshot: SnapshotToken;
  requestGeneration: number | null;
  items: Array<{
    label: string; insertText: string; kind: "content" | "class" | "asset";
    identity: ContentIdentity | null; detail: string | null;
  }>;
  complete: boolean; // underlying inventory for the requested kind is known complete
  isIncomplete: boolean; // result cap reached OR inventory is incomplete/unknown
}
```

Narrow `query` to find omitted matches; there is no unbounded/paged catalog export here.
Classes use the existing reflection class index; assets use `assets_of_class` and its
completeness evidence. Content insertion prefers a fully qualified generated object path.
Ordinary `textDocument/completion` retains existing providers, caps both array/list responses
at 200, and rejects results superseded by edits/config changes.

## Q16: content-ID rename

Call `kdf/exportSnapshot`, then `kdf/renameContent`:

```ts
{
  snapshot: SnapshotToken; // required
  textDocument: {uri: string; version: number | null}; // version required
  position: Position; // declaration or an unambiguous typed reference
  newName: string; // ASCII letter, then letters/digits/underscore/dot/hyphen
}
```

Response:

```ts
{
  schemaVersion: 1; snapshot: SnapshotToken;
  identity: ContentIdentity; newIdentity: ContentIdentity;
  documents: Array<DocumentSnapshot & {
    edits: Array<{range: Range; expectedText: string; newText: string}>;
  }>;
  workspaceEdit: {documentChanges: TextDocumentEdit[]};
}
```

Every edit is verified against its exact text/range. Open documents carry LSP versions;
closed documents carry null versions plus required digests in `documents`. Preserve all
these guards in the extension's preview. Immediately before applying, request the plan again
with the original snapshot; stale plans must be discarded. Check current document versions,
digests and expected text before applying the returned versioned `WorkspaceEdit`. Closed-file
digests require host verification because LSP's null version alone cannot protect them.
No API can make a read-only plan atomic with subsequent external filesystem writes.

The plan updates only declarations and typed content references. It refuses duplicate
normalized identities, destination collisions, ambiguous/unmapped operands, dynamic values,
missing source mappings, scripts/includes/redirects/aliases and incomplete inventories.
Only class/asset namespaces support rename. Pack refs, config field IDs and other identity
namespaces refuse. Referenced categories inside config documents can be updated, but config
IDs are never renamed. Pack manifest version/redirect entries are not edited.

Standard `prepareRename`/`rename` use this engine. The client must declare
`workspace.workspaceEdit.documentChanges: true`; otherwise prepare returns null and rename
returns `-32602`. This is a deliberate safety restriction on the former unversioned rename.
The method remains standard LSP; the former automatic manifest version bump and redirect
insertion are replaced by explicit content-only edits. Old class/navigation queries are unchanged.

## Q17: reflection replacement evidence

No edit is inferred from similar spelling, native/Blueprint `remapsTo`, or a reflection diff.
Only the bound archive can supply optional replacement metadata in its reflection manifest:

```json
{
  "replacementMetadata": {
    "schemaVersion": 1,
    "replacements": [
      {"kind":"class","oldName":"/Script/Test.Old","newName":"/Script/Test.New","authority":"unreal-core-redirects","verified":true},
      {"kind":"property","owner":"/Script/Test.New","oldName":"mOld","newName":"mNew","authority":"unreal-core-redirects","verified":true}
    ]
  }
}
```

This is an optional exporter evidence contract, not pack YAML syntax. Existing exporters are
not assumed to emit it. The configured archive is the trust boundary: v1 metadata must assert
`verified:true` from `unreal-core-redirects`, identify exactly one replacement, and point to an
existing destination in a complete reflection snapshot. Conflicts, redirect chains, unsupported
versions, unverified/unknown authorities, missing targets, nested/escaped source ranges, or
limited exports yield `unknown` with no replacement. Metadata is capped at 4,096 records.

Diagnostics retain existing fields and add optional core `evidence`. Published LSP diagnostic
`data` additionally carries `{schemaVersion:1,explanationId,evidence}`, preserving existing
`yamlPath`, `document`, and `schema`. Code actions recompute evidence against the current
buffer/bound reflection; client-supplied metadata cannot authorize edits. Reflection quick fixes
require an open document and `workspaceEdit.documentChanges` support, and carry its exact version.

```ts
type Evidence = {
  schemaVersion: 1; status: "verified" | "unknown"; reason: string;
  reflectionDigest: string | null;
  replacement: null | {
    kind: "class" | "property"; oldName: string; newName: string;
    owner: string | null; authority: "unreal-core-redirects"; reflectionDigest: string;
  };
};
```

## Effective schema and Q21 explanations

`kdf/effectiveSchema(Guards & {kind: string})` returns
`{schemaVersion:1,snapshot,kind,schema,resources}`. `resources` maps schema `$id` URLs to the
actual locally loaded schema documents, including enabled project/user overrides. Resolve
references from this map; no network retrieval is required.

`kdf/explain` and alias `kdf/why` accept
`Guards & {ruleId?: string, explanationId?: string, position?: Position}`. File/position/rule/ID
filters are optional. Both run the same lint engine against captured buffers and return:

```ts
{
  schemaVersion: 1; snapshot: SnapshotToken; positionEncoding: "utf-16";
  explanations: Array<{
    schemaVersion: 1; id: string; ruleId: string; message: string; source: string;
    file: string | null; document: number | null; yamlPath: string; range: Range;
    evidence: Evidence; action: string;
  }>;
}
```

Explanation IDs hash the rule, source location, message and evidence; reordering diagnostics
does not change them. Unknown evidence remains explicitly unknown. Explanations do not invent
runtime behavior beyond the lint evidence.

## Q20/Q21 CLI contracts

```sh
kdf lint PATH --format json
kdf lint PATH --format sarif
kdf why PATH --format json [--rule CODE] [--line ONE_BASED_START_LINE]
kdf why PATH --format text
```

`why` accepts the same datapack/profile/schema/settings/exit-policy flags as `lint`.
Its default format is JSON; only `json` and `text` are valid for `why`.

JSON lint retains `ok,packs,documents,errors,warnings,diagnostics`, adds `schemaVersion:1` and
`positionEncoding:"utf-16"`. Each diagnostic retains its old fields and adds
`ruleId,location:{uri,document,yamlPath,range},evidence,explanationId`.

SARIF 2.1.0 uses sorted stable `ruleId`/`ruleIndex`, escaped file URIs, one-based regions,
`columnKind:"utf16CodeUnits"`, and the same location/evidence/explanation ID under result
`properties`. `partialFingerprints["kdfEvidence/v1"]` is the explanation ID. Advisory/hint
findings use SARIF `note` and never count towards warnings-as-errors. Empty/invalid end ranges
are omitted. `kdf-cli` exports `diagnostic_record`, `json_report`, `sarif_report`, `why_report`,
and `lint_exit_code` for Rust consumers.

`why` JSON is `{schemaVersion:1,positionEncoding:"utf-16",explanations:[...]}` using the same
explanation records as LSP. Exit status for both lint/why: 0 for no errors (or warnings under
normal policy), 1 for lint errors or counted warnings with warnings-as-errors, 2 for usage,
settings or schema configuration failure, 3 for an internal panic. Filtering `why` output does
not change the underlying lint exit result. `ok` continues to mean zero errors, independent of
warnings-as-errors.

## Pack manifest grammar

`scripts` is a sequence with required `language` and portable `entry`. Supported fields are
`languageVersion` (1..4294967295, default 1), `realm` (any/server/client), `network`
(any/standalone/networked), `authority` (any/required/forbidden), `reflectionClasses` or its
exclusive alias `classes` (max 64 unique paths), and `imports` (max 64). Imports require
`key,kind,path` plus `sha256` or alias `digest`; kinds are yaml/structured/text/binary/image/audio.
Portable paths reject traversal, Windows device names, empty segments and case-folded import
collisions. KDataForgeJS requires `entry: scripts/main.ts`. Explicit `id`, `capabilities` and
unknown extension-only keys are rejected. `contributer` remains optional at runtime.

The copied runtime TypedStarter manifest is tested. The extension SDK template is valid after
rendering its `{{ref}}` placeholder. The upstream runtime sample's separate `example.yml` is
import data without a root type: whole-pack lint still reports `semantic.missing-type`, matching
the runtime loader's skip/warning behavior; validation does not silently invent a document type.
