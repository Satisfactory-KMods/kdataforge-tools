# @sf-kmods/kdataforgejs

TypeScript types for KDataForgeJS packs. They describe the built-in `@sf-kmods/kdataforgejs` module that the runtime
provides to a pack (`ctx`, `defineMeta`, `defineHandler`, `Context`, ...) and ship a strict `tsconfig` base.
Use together with [`@sf-kmods/kdataforgejs-cli`](../cli/README.md); `kdf init` sets everything up.

```sh
npm install --save-dev @sf-kmods/kdataforgejs
```

```jsonc
// tsconfig.json
{ "extends": "@sf-kmods/kdataforgejs/tsconfig.base.json", "include": ["DataForge/**/*.ts"] }
```

```ts
// DataForge/scripts/main.ts
import { ctx, defineHandler, defineMeta } from "@sf-kmods/kdataforgejs";

export const meta = defineMeta({ capabilities: [] });

export default defineHandler(async () => {
    console.info(ctx.catalog().digest);
});
```

Import `ctx` in any script module to use the same context without passing it through helpers.
It belongs to that script realm; the previous `defineHandler(async (ctx) => { ... })` form remains supported.

This package is **editor tooling only**. Packs never contain `package.json` or `node_modules`; the runtime resolves
`@sf-kmods/kdataforgejs` itself. The declarations are copied from the KDataForgeJS plugin at build time.

Use value imports for closed choices: `ImportKind.Yaml`, `Capability.ImportsRead`, `HookPhase.After`,
`DocumentType.Cdo`, `PropertyOperation.Set`, and the other enums exposed by completion. Raw strings no longer satisfy
these TypeScript parameters; YAML values are unchanged. Enum values are frozen runtime exports, so `import type`
alone cannot supply them. Update the runtime, SDK, CLI and VS Code extension together and refresh `kdf game-data`.

The package includes `enums.kdfjs.json`, `authoring.kdfjs.json` and the declaration integrity manifest.
