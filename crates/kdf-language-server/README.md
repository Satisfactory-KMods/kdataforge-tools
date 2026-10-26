# kdf-language-server

Language server for KDataForge Tools (stdio, `tower-lsp`). It is host neutral: VS Code and any LSP
client talk to the same binary. Validation, schemas and reflection queries come from `kdf-validation`.

## Host inputs

Everything here is optional. Without it the server behaves as before: the datapack comes from `.kdf-editor.toml`
(`[datapack] path`) or `KDF_EFFECTIVE_DATAPACK_PATH`, the user settings from `KDF_USER_SETTINGS_PATH`, and the editor
commands described below stay on.

### `initializationOptions` (and `workspace/didChangeConfiguration` settings)

```json
{ "kdf": { "datapackPath": "C:/path/vanilla.kdfdata", "userSettingsPath": "C:/path/settings.toml", "clientCommands": true } }
```

| key                | meaning                                                                                                                                                      |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `datapackPath`     | Reflection datapack to use. Wins over `KDF_EFFECTIVE_DATAPACK_PATH` and `.kdf-editor.toml`. An empty string selects schema-only validation.                 |
| `userSettingsPath` | User `settings.toml`. Wins over `KDF_USER_SETTINGS_PATH`; the project `.kdf-editor.toml` still overrides it per key.                                         |
| `clientCommands`   | Default `true`. Completion items re-trigger the editor with `editor.action.triggerSuggest` / `editor.action.triggerParameterHints`. `false` omits them.      |

`workspace/didChangeConfiguration` accepts the same object (`settings.kdf`, or the keys directly in `settings`). A push
replaces `datapackPath` and `userSettingsPath` as a whole; `clientCommands` changes only when present. The desktop client sends
`settings: {}`, which changes nothing and re-reads `.kdf-editor.toml`.

### Client capabilities that change the answers

| capability                                                     | effect when declared                                              | fallback                          |
| -------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------- |
| `completionItem.documentationFormat` contains `markdown`       | completion documentation is markdown                              | plain string                      |
| `completionItem.resolveSupport.properties` has `documentation` | cards are built lazily by `completionItem/resolve` (`data`)       | cards are sent inline             |
| `completionItem.snippetSupport`                                | function items insert a snippet with the cursor between `(` `)`   | plain text                        |
| `hover.contentFormat` contains `markdown`                      | hover is markdown                                                 | `plaintext` markup                |
| `signatureInformation.documentationFormat` contains `markdown` | signature documentation is markdown                               | plain string                      |
| `parameterInformation.labelOffsetSupport`                      | parameter labels are `[start, end]` offsets                       | the parameter text as a string    |

## Behaviour a host can rely on

- **Document identity**: open buffers are keyed by a normalized path (case-insensitive drive letter, percent-decoded), so
  `file:///c%3A/...` and `file:///C:/...` are the same document. Diagnostics are published only under the URL the client used
  for the document. Locations and edits for open documents use the client's URL as well.
- **Incremental sync**: positions past the end of a line are clamped to the line end, lines past the end to the document end.
- **Completion lists** are capped at 200 items; `isIncomplete` is `true` only when the cap omits matching results. What is
  typed narrows members and `path:` properties on the server before the cap applies. Items carry a short `detail`; the card
  is `documentation` inline or, for clients that declared `resolveSupport`, `completionItem/resolve` with the item's `data`.
- **Latency model**: requests answer from the last good workspace snapshot and the loaded reflection index. A snapshot is
  refreshed in the background (one rebuild at a time, edits coalesced, only changed files re-read; files changed on disk are
  noticed within two seconds, or on `workspace/didChangeWatchedFiles`). Definition and references refresh the snapshot first
  (cheap, incremental) so they see the files as they are. The reflection archive is loaded once per file version and warmed
  on a worker thread; an unbound datapack is dropped from memory.
- **Validation** runs in one task: edits within the debounce window collapse into one run on the newest buffers.
- `.kdf-editor.toml` and the datapack file are looked at most every 500 ms; `didChangeConfiguration` and watched-file events
  invalidate that cache at once.

## Custom requests

`kdf/classes`, `kdf/classInfo`, `kdf/operations`, `kdf/previewBinding` (compares datapack bindings without touching project
files) and `kdf/revalidate` (runs a lint even when `[lint] trigger = "manual"`).
