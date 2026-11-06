# KDataForge Tools

Language support, linting, Codex and typed KDataForgeJS authoring for VS Code.

## Commands

- **KDataForge: Open Codex** searches reflected classes.
- **KDataForge: Insert Class Reference** inserts a selected class path.
- **KDataForge: Lint File/Project** runs the bundled Rust validator.
- **KDataForge: Switch Datapack** binds a `.kdfdata` export.
- **KDataForgeJS: Create Project Here/Create Script** creates typed authoring files.

YAML completion, hover and diagnostics accept both `properties:` lists and mappings. Mapping keys
are property paths; a plain value means `set`. An indented `$op: append` with `value:` selects another
operation. Nested instanced objects use the same syntax. The YAML snippets `kdf-properties-map`,
`kdf-property-op` and `kdf-unlock-recipe-map` insert starting examples.

Without a datapack the language server stays in schema-only mode. Runtime-dependent conclusions remain
unknown instead of being treated as valid.

Licensed under the MIT License.
