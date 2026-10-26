import { existsSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { checkProject } from "./check.js";
import { inspectSyncDrift } from "./sync.js";
import { validateProject } from "./validate.js";
import { lintProject } from "./lint.js";
import { detectGameInstallations, resolveGameInstallation } from "../game/installations.js";
import { hasErrors, type Diagnostic } from "../diagnostics.js";
import { discoverPacks } from "../pack/discover.js";
import { inspectGameData } from "./gameData.js";

export type HealthStatus = "pass" | "warn" | "fail" | "unknown";
export interface HealthCheck {
	readonly name: string;
	readonly status: HealthStatus;
	readonly message: string;
}

/** Read-only health report. Unknown means the check could not be run. */
export function doctorProject(directory: string, gamePath?: string, packRef?: string): HealthCheck[] {
	const root = resolve(directory);
	const checks: HealthCheck[] = [];
	checks.push({ name: "node", status: Number(process.versions.node.split(".")[0]) >= 20 ? "pass" : "fail",
		message: `Node ${process.versions.node} (requires >=20)` });
	if (!existsSync(root)) {
		checks.push({ name: "project", status: "fail", message: `Project does not exist: ${root}` });
		return checks;
	}
	for (const [name, run] of [
		["validate", () => validateProject(root, packRef)],
		["check", () => checkProject(root, undefined, packRef)],
		["lint", () => lintProject(root, packRef)],
	] as const) {
		try {
			const diagnostics = run();
			checks.push(fromDiagnostics(name, diagnostics));
		} catch (error) {
			checks.push({ name, status: "unknown", message: error instanceof Error ? error.message : String(error) });
		}
	}
	try {
		const drift = inspectSyncDrift(root, packRef);
		const changed = [...drift.changedPacks, ...drift.changedTypes];
		checks.push({ name: "manifest", status: changed.length === 0 ? "pass" : "warn",
			message: changed.length === 0 ? "Manifest and config types synchronized" : `Run kdf sync: ${changed.map((path) => relative(root, path)).join(", ")}` });
	} catch (error) {
		checks.push({ name: "manifest", status: "unknown", message: error instanceof Error ? error.message : String(error) });
	}
	try {
		const installations = gamePath ? [resolveGameInstallation(gamePath)] : detectGameInstallations();
		checks.push({ name: "installation", status: installations.length > 0 ? "pass" : "warn",
			message: installations.length > 0 ? installations.map((item) => item.root).join(", ") : "No modded Satisfactory installation found" });
	} catch (error) {
		checks.push({ name: "installation", status: "unknown", message: error instanceof Error ? error.message : String(error) });
	}
	try {
		const selection = discoverPacks(root, packRef);
		const projects = [...new Set(selection.packs.map((pack) => pack.projectRoot))];
		if (selection.workspace) {
			const rootLocks = ["package-lock.json", "pnpm-lock.yaml"].filter((name) => existsSync(join(root, name)));
			const nested = projects.filter((project) => project !== root &&
				["package-lock.json", "pnpm-lock.yaml"].some((name) => existsSync(join(project, name))));
			checks.push({ name: "lockfile", status: rootLocks.length > 1 ? "fail" : rootLocks.length === 0 || nested.length > 0 ? "warn" : "pass",
				message: rootLocks.length > 1 ? "Both npm and pnpm root lockfiles exist" : nested.length > 0
					? `Nested lockfiles: ${nested.map((path) => relative(root, path)).join(", ")}`
					: rootLocks.length === 0 ? "No root lockfile found" : `Root lockfile: ${rootLocks[0]}` });
		}
		const health = inspectGameData(root, projects, gamePath);
		checks.push({ name: "game-data", status: health.status === "current" ? "pass" : health.status === "stale" ? "warn" : "unknown",
			message: health.message });
		for (const project of projects) {
			const types = join(project, "DataForge/typescript/kdataforgejs/index.d.ts");
			checks.push({ name: `reflection-types:${relative(root, project) || "."}`, status: existsSync(types) ? "pass" : "warn",
				message: existsSync(types) ? `Types available: ${types}` : `Generated types missing: ${types}` });
		}
	} catch (error) {
		checks.push({ name: "game-data", status: "unknown", message: error instanceof Error ? error.message : String(error) });
		checks.push({ name: "reflection-types", status: "unknown", message: error instanceof Error ? error.message : String(error) });
	}
	return checks;
}

function fromDiagnostics(name: string, diagnostics: readonly Diagnostic[]): HealthCheck {
	const status = hasErrors(diagnostics) ? "fail" : diagnostics.length > 0 ? "warn" : "pass";
	return { name, status, message: diagnostics.length === 0 ? "Passed" : diagnostics.map((item) => `${item.code}: ${item.message}`).join("; ") };
}
