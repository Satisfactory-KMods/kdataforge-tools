# @sf-kmods/kdataforgejs-cli

`kdf` sets up, checks and validates KDataForge projects that use KDataForgeJS (TypeScript) scripts.

```sh
npx @sf-kmods/kdataforgejs-cli init MyPack
cd MyPack
npm install
npm run verify
```

## Commands

| Command | What it does |
| --- | --- |
| `kdf init [dir]` | Creates a project: `package.json`, `tsconfig.json` and a `DataForge/` pack with `pack.yml` and `scripts/main.ts`. `--name` sets the pack ref, `--force` writes into a non-empty folder. |
| `kdf check [dir]` | Type-checks the project against `@sf-kmods/kdataforgejs` with the bundled TypeScript. Nothing is emitted. |
| `kdf format [dir]` | Formats JavaScript and TypeScript files below `DataForge/scripts/` with bundled Biome. |
| `kdf lint [dir]` | Lints JavaScript and TypeScript files below `DataForge/scripts/` with bundled Biome recommended rules. |
| `kdf validate [dir]` | Checks `pack.yml`, the entry file, the static import graph and the digests of declared imports. |
| `kdf sync [dir]` | Generates sorted `imports`, SHA-256 digests, `reflectionClasses`, and pack-local config key/value types. |
| `kdf hash [dir]` | Rewrites stale `sha256` values of declared imports and leaves your comments alone. |
| `kdf game-data [dir]` | Runs manifest sync, finds a modded Satisfactory installation, reuses a valid cached export or starts a full reflection export, closes the game and imports the archive plus generated TypeScript declarations. Use `--game-path <root-or-executable>` to select an installation explicitly; add `--content-types` for schema-derived content document types and `--force` to bypass the cache. |
| `kdf doctor [dir]` | Read-only health report for Node, project checks, manifest drift, installation and imported game data. `--json` emits machine-readable statuses: `pass`, `warn`, `fail`, `unknown`. |
| `kdf dev [dir]` | Watches authored files, debounces changes, then runs sync, check, lint and validate in order. After success, prints manual `KDF.Reload` instruction. |
| `kdf add <handler\|hook\|item\|recipe\|document> <name> [dir]` | Creates one scaffold and updates entry imports, handler call and literal capabilities. Existing destinations and computed metadata are refused. Documents default to `cdo`; `--type item` or `--type recipe` selects another minimal document form. |
| `kdf update [dir]` | Read-only, network-free report of declared and locally installed SDK/CLI versions and game-data status. `--apply` updates local dev dependencies, then runs the newly installed local CLI's `game-data` once. `--offline --apply` skips the package manager and refreshes from installed packages/cache/game only. |

Exit code `0` means all good, `1` means findings, `2` means the command was used wrong.

`validate` repeats the checks the game does when it loads a pack, so most mistakes show up before you start Satisfactory.
It does not replace the runtime: catalog, authority and limits are still decided in game.

The generated project keeps `package.json` and `tsconfig.json` next to the `DataForge` folder, never inside it. The pack
itself has no `node_modules`. `npm run refresh` runs `kdf game-data`; `npm run fix` formats scripts and synchronizes
generated manifest fields. `format`, `lint`, `check`, `validate`, `hash`, `sync`, `test`, and `verify` are available too.

## Multiple packs in one workspace

Keep each pack in its own tooling project, for example `packs/Alpha/DataForge/pack.yml` and
`packs/Beta/DataForge/pack.yml`. At the repository root, declare npm `workspaces: ["packs/*"]`
or pnpm `packages: ["packs/*"]` in `pnpm-workspace.yaml`. Run `kdf init packs/Alpha --name Alpha`
for each pack; `package.json`, `tsconfig.json` and dependencies remain beside that pack's
`DataForge/`, never inside it. Install dependencies from the workspace root. Keep one root
`package-lock.json` or `pnpm-lock.yaml`; the CLI never installs packages during discovery,
checks, `doctor`, or `--offline` update.

From workspace root, `check`, `lint`, `validate`, `sync`, `hash`, `format`, `doctor`, and `dev`
cover all discovered packs. Add `--pack Alpha` to select one unique pack ref. `add` requires
`--pack` when several packs are present. Only literal workspace paths and one whole `*`
directory segment are supported; unsupported patterns, escaping symlinks, and duplicate refs
fail explicitly. `check` uses each pack project's own tsconfig, keeping config/reflection types
isolated. Packs still cannot import code outside their own `scripts/` or carry `node_modules`.

`kdf game-data` from workspace root exports the installed game once, uses one root cache/archive,
and installs generated reflection declarations plus `tsconfig.kdf.json` into every pack project.
The export describes installed game/mod content; authored workspace packs appear only after they
are installed into the game. `--pack` is intentionally unavailable for `game-data` and `update`.
Manifests are synchronized after successful game-data import. The CLI stages generated files,
uses a workspace writer lock under `.kdataforge/`, and restores old files on ordinary install
failure. A crash during a multi-file swap can leave mixed files; rerun `kdf game-data` and
`kdf sync`. Check for a live writer before removing a stale `writer.lock`.

## Update SDK and game data

`kdf update` reports versions from each project's manifest and installed `node_modules`; it
does not query a registry or modify files. It compares the installed archive with a validated
cache for the current game/save when possible. `unknown` freshness is never reported as current.
Run `kdf update --apply` to invoke detected npm/pnpm with fixed
`@sf-kmods/kdataforgejs@latest` and `@sf-kmods/kdataforgejs-cli@latest` dev dependencies,
`--ignore-scripts`, and no global install. It then runs the newly installed local CLI's
`game-data` once. Pass `--game-path`, `--timeout`, `--content-types`, or `--force` through to
that refresh. Package-manager network and lockfile operations are not atomic across packs:
if one install fails, inspect root lockfile and package manifests before retrying. Generated
reflection files remain untouched until game-data succeeds. `kdf update --offline --apply`
requires both packages already installed at matching versions, skips package-manager registry
access, and only refreshes game data. Offline mode never alters lockfiles.

Literal `ctx.import(kind, path)` calls and reflected paths used by `ctx.getClass`, `ctx.getObject` and
`ctx.getDataAsset` are source of truth. YAML imports are scanned for `target` paths. Run `kdf sync`; do not maintain
manifest import digests or reflection class lists by hand. Dynamic import paths are rejected because CLI cannot grant
their authority safely.

## Import game data

Run from the project directory:

```sh
kdf game-data
```

The command searches Steam libraries and Epic manifests on Windows. On Linux it searches native and Flatpak Steam
libraries and launches app `526870` through Steam. Only installations containing both KDataForge and KDataForgeJS are
eligible. To bypass discovery:

```sh
kdf game-data --game-path "C:\\Games\\Satisfactory"
```

The CLI loads the newest local save automatically, starts a full reflection export, and exits the game when publication completes. The CLI waits up to 15
minutes by default; override this with `--timeout <seconds>`. It writes `.kdataforge/satisfactory.kdfdata`, imports
generated declarations below `DataForge/typescript/kdataforgejs/`, creates `tsconfig.kdf.json`, and makes later
`kdf check` calls use those installed-game types automatically. `.kdataforge/` is added to `.gitignore` because the
archive can be large. Declarations are split into `runtime.d.ts`, `globals.d.ts`, alphabetical `satisfactory/` groups,
and Unreal-path `paths/` groups behind `index.d.ts`. Biome formats generated TypeScript and JSON through its JavaScript
API. Projects created by `kdf init` also ignore `DataForge/typescript/kdataforgejs/`; regenerate it for the local game
and mod set. Temporary export and staging directories are removed after the game exits.

`kdf game-data --content-types` opts into schema-derived document entries for `applyDocument`, including schematics,
research, unlocks, buildings and DataAssets. Field names and structural types are checked locally; runtime validation
still enforces ranges, conditional rules, reflection, capabilities and lifecycle. Export without the flag to restore
generic content types. Both modes work on cache hits without relaunching the game. The schemas ship in the CLI package; generation does not fetch schema URLs.

`kdf sync` writes `DataForge/typescript/kdataforgejs-config.d.ts` from authored `*.config.yml`/`*.config.yaml` fields.
These keys type `ctx.config.get` and `onChange`; runtime access still requires `kdf.config.read`. Regenerate the file
after changing config definitions. Keep generated files outside your source edits.

For CLI maintenance, `pnpm run sync:schemas` refreshes the checked-in `schemas/` snapshot from the workspace's
`crates/kdf-validation/schemas/`. Normal builds and installed packages use the bundled snapshot and
do not require a separate editor checkout.

Complete exports are cached in `.kdataforge/cache/<input-sha256>/<output-sha256>/` with archive, declarations and a hash receipt.
The input fingerprint includes platform, installation path, game executable, save, installed mod descriptors and binaries,
base-game package metadata, loose installed `DataForge` pack files (including scripts and imports), and cache/tool schema.
Loose pack hashing is limited to 10,000 files or 512 MiB; larger inputs skip caching. A validated hit skips launch; missing or corrupt artifacts trigger a
fresh export. `--force` bypasses the hit. The game-data archive and declarations still install into normal project paths.

`kdf dev` watches project configuration and authored `DataForge/` files. Generated declarations and manifests from its
own sync do not trigger another run. No automatic game command is sent: run `KDF.Reload` in the local game/server console
after successful checks. Item and recipe scaffolds contain placeholder definitions; fill in actual parent classes,
ingredients, products and machines before loading them. Hook scaffolds leave target selection for the author.
