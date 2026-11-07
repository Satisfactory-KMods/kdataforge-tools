import { ctx, defineHandler, defineMeta } from "@sf-kmods/kdataforgejs";
import { describeCatalog } from "./lib";

export const meta = defineMeta({ capabilities: [] });

export default defineHandler(async () => {
	console.info(describeCatalog(ctx.catalog().digest));
	await Promise.resolve();
});
