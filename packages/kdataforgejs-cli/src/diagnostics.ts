export interface Diagnostic {
	readonly severity: "error" | "warning";
	readonly code: string;
	readonly file: string;
	readonly message: string;
}

export function formatDiagnostic(diagnostic: Diagnostic): string {
	return `${diagnostic.file}: ${diagnostic.severity} ${diagnostic.code}: ${diagnostic.message}`;
}

export function hasErrors(diagnostics: readonly Diagnostic[]): boolean {
	return diagnostics.some((diagnostic) => diagnostic.severity === "error");
}
