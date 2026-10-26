# KDataForge Tools

Public tooling for authoring KDataForge packs in VS Code.

- VS Code language support, diagnostics and quick fixes
- Rust validator and `kdf` CLI
- Codex class lookup backed by `.kdfdata` reflection exports
- KDataForgeJS types, CLI, snippets and project actions

Extension ID: `kmods.kdataforge`.

## Channels

| Branch | VS Code Marketplace | npm packages |
| --- | --- | --- |
| `dev` | Experimental pre-release | prerelease version, npm dist-tag `beta` |
| `main` | Stable release | stable version, npm dist-tag `latest` |

Versions follow KMods: `<year>.<calendar quarter>.<GitHub run number>`. Development npm versions add
`-beta.0`. Source manifests use `9999.9.9` until CI applies an exact release version.

## Build

```sh
pnpm install --frozen-lockfile
pnpm build
pnpm test
cargo test --workspace
```

## Release

CI validates pushes and pull requests. The manual release workflow uses the selected branch as the
channel contract. `dev` can publish only experimental/beta artifacts; `main` can publish only stable
artifacts. Publication requires protected GitHub environments and their scoped credentials.

Licensed under the [MIT License](LICENSE).
