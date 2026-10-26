import { defineHandler } from "@sf-kmods/kdataforgejs";
import { describeCatalog } from "./lib";

export default defineHandler(async (ctx) => {
	console.info(describeCatalog(ctx.catalog().digest));
	await Promise.resolve();
});
