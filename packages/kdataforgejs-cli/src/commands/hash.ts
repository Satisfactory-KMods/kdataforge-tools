import { join } from "node:path";
import { discoverPacks } from "../pack/discover.js";
import { updatedImportDigests } from "../pack/imports.js";
import { commitStaged, stageTexts, withProjectLock } from "../pack/transaction.js";

/** Updates sha256 of declared script imports; returns one `pack: key` line per changed import. */
export function hashProject(projectRoot: string, packRef?: string): string[] {
	discoverPacks(projectRoot, packRef);
	return withProjectLock(projectRoot, () => {
		const updates = discoverPacks(projectRoot, packRef).packs.map((pack) => ({ root: pack.root, ...updatedImportDigests(pack.root) }));
		commitStaged(stageTexts(updates.filter((item) => item.changed.length > 0)
			.map((item) => ({ path: join(item.root, "pack.yml"), content: item.content }))));
		return updates.flatMap((item) => item.changed.map((key) => `${item.root}: ${key}`));
	});
}
