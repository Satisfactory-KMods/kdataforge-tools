# @sf-kmods/kdataforgejs

TypeScript types for KDataForgeJS packs. They describe the built-in `@sf-kmods/kdataforgejs` module that the runtime
provides to a pack (`defineMeta`, `defineHandler`, `Context`, ...) and ship a strict `tsconfig` base.
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
import { defineHandler, defineMeta } from "@sf-kmods/kdataforgejs";

export const meta = defineMeta({ capabilities: ["kdf.items.define"] }); // optional
```

This package is **editor tooling only**. Packs never contain `package.json` or `node_modules`; the runtime resolves
`@sf-kmods/kdataforgejs` itself. The declarations are copied from the KDataForgeJS plugin at build time.
