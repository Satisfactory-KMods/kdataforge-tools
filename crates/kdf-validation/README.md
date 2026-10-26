# KDataForge validation core

`kdf-validation` supplies one rule engine to `kdf lint`, `kdf-language-server`, and datapack switch previews. It embeds the 22 versioned JSON Schemas (19 canonical root types, `pack.yml`, the `mam` alias and the shared `common` definitions). Runtime mode treats `contributer` as project policy; `--profile legacy` retains old policy checks for migration comparisons.

## Lint settings

User `settings.toml` is selected by `KDF_USER_SETTINGS_PATH` or CLI `--user-settings`. Project `.kdf-editor.toml` overrides user values per key. Relative schema paths resolve from the file's scope directory. Disabled directories remain in TOML but are excluded from live lint.

```toml
[lint]
trigger = "onType" # onType | onSave | manual
debounce_ms = 300
schema_dirs = [{ path = "ci/schemas", enabled = false }]
allow_schema_override = false
allow_unknown_types = false
warnings_as_errors = false
merge_duplicate_diagnostics = true
output_format = "text" # text | json | github | sarif
coldProperties = "info" # info | hint | off: how `property.cold` is reported
```

`allow_unknown_types` changes unknown root types from errors to warnings. `warnings_as_errors` changes CLI exit status; published LSP diagnostic severity stays a warning. Schema and semantic diagnostics merge only for known equivalent rule pairs at the same YAML location. Legacy profile keeps separate diagnostics for comparison.

LSP reads settings at startup and on `workspace/didChangeConfiguration` or watched file changes. A host can pass the datapack, the user `settings.toml` and the editor-command switch as `initializationOptions.kdf` (`datapackPath`, `userSettingsPath`, `clientCommands`; see the `kdf-language-server` README); the environment variables stay the fallback. `kdf/revalidate` runs explicit lint in manual mode. Only `.kdfdata` archives with supported reflection schema version are accepted. `kdf preview-datapack <root> <candidate.kdfdata>` compares bindings without changing project files; LSP `kdf/previewBinding` also includes open unsaved buffers.

## Pack configuration

`<name>.config.yml` documents (root type `config`) define typed, player-editable pack settings. The core validates the definitions, `cfg:`/`session:`/`ags:`/`option:` references, `{prefix:body}` templates, `config:` conditions (pack, document and element level), hot/cold usage, cold properties and the pack icon. Sources of truth: the runtime YAML contract in the KDataForge docs, `tests/fixtures/config-contract-vectors.json` (expression, template and element-condition vectors) and `tests/fixtures/diagnostic-codes.json` (diagnostic catalogue). `API-CONFIG.md` lists the public API the language server builds on.

`property.cold` is informational: it keeps `severity: warning` for consumers that only know errors and warnings, carries `advisory: "information" | "hint"`, and is excluded from `LintResult::warnings`. Reflection-dependent checks (members, call allowlist, cold properties, settings) run only when a datapack is bound; the datapack may carry the optional sections `indexes/settings.json` and `catalog/cold_properties.json`.
