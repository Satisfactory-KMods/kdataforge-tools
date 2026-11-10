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

## Mod-owned schemas

A mod can provide `Resources/KDataForge/Schemas/<type>.schema.yml` at its mod root.
For a workspace or document, the validator searches ancestor mod roots and the direct children
of `Mods`, `Mods/GameFeatures`, `FactoryGame/Mods`, and `tools` at each ancestor. Junctions
are resolved for de-duplication. It does not search pack document trees. A schema uses the
existing `$id` and `x-kdf-type` format; the filename convention for documents remains
`<name>.<type>.yml`. Schema `x-kdf-stage`, if supplied, remains available in schema exports;
it does not register a runtime handler or alter runtime ordering.

Discovered paths sort by canonical path. The first schema for a root type wins;
`schema.duplicate-type` names both files. A mod schema claiming a built-in type is ignored
and produces `schema.builtin-shadow`. Explicit `schema_dirs` are loaded afterwards and can
replace a discovered or built-in schema when `allow_schema_override` is enabled.
`schema.unknown-type` remains an error by default when no schema provides a type.
Ensure the schema is distributed as a loose file at this path. CSS DLC staging reads
`[StageSettings]` in `Config/PluginSettings.ini`; `+AdditionalNonUSFDirectories = Resources`
stages the directory recursively as NonUFS. SatisfactoryPlus already has that entry.

## Third-party condition providers

Mods can declare condition providers in `<ModRoot>/Resources/KDataForge/providers.yml`:

```yaml
providers:
  - prefix: sfp
    description: SatisfactoryPlus module availability.
    valueType: bool
    freeFormKeys: true
```

`valueType` (`bool`, `number`, or `string`) and `freeFormKeys` are optional. The latter is
metadata; tooling does not enumerate or validate the mod's keys. Declarations are static tooling
hints, not runtime registration. Exact spelling matters. Sorted canonical paths determine the
first winner; duplicates and built-in claims report `provider.duplicate-prefix` and
`provider.builtin-shadow`. Unknown expression providers report `template.unknown-provider`.
The runtime remains authoritative about provider registration and values.
