# {{ref}}

KDataForge project with a KDataForgeJS (TypeScript) script, created with `kdf init`.

| Path | Purpose |
| --- | --- |
| `DataForge/` | The pack. Copy it to `FactoryGame/DataForge/<name>/` or ship it with your mod. |
| `DataForge/pack.yml` | Pack manifest with script entry, optional realm policy, imports, and reflected classes. |
| `DataForge/scripts/main.ts` | Canonical entry with default `defineHandler` and optional capability `meta`. |
| `package.json`, `tsconfig.json` | Editor tooling only. Never put them inside `DataForge/`. |

```sh
npm install
npm run refresh    # launch game and refresh archive plus generated game types
npm run doctor     # read-only project and installation health
npm run dev        # watch authored files; run KDF.Reload manually after checks pass
npm run sync       # generate imports, hashes and reflection classes from source usage
npm run check      # type-check against @sf-kmods/kdataforgejs
npm run format     # format scripts with bundled Biome
npm run lint       # lint scripts with bundled Biome rules
npm run validate   # pack.yml, entry shape, static imports, import digests
npm run hash       # refresh sha256 of declared imports after editing them
npm run fix        # format scripts and synchronize generated manifest fields
npm run verify     # sync, type-check, lint and validate
kdf update         # inspect installed SDK/CLI versions and game-data freshness
kdf update --apply # update local dev dependencies, then refresh game data
```

For a multi-pack repository, create each project with `kdf init packs/<name> --name <ref>`.
Declare `packs/*` in root npm `workspaces` or `pnpm-workspace.yaml`, and install once from
workspace root. Root commands cover every pack; use `--pack <ref>` to target one for checks,
formatting or scaffolding. `kdf game-data` refreshes all packs from one installed-game export.
`kdf update --offline --apply` uses packages already installed and leaves lockfiles alone.

Use `npx kdf add item Example` (or `recipe`, `handler`, `hook`, `document`) to create a scaffold.
Edit placeholder item/recipe values before loading the pack. `npx kdf game-data --force` rebuilds cached game data.
