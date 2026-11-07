import { invoke, isTauri } from "@tauri-apps/api/core";

type DiagnosticKind = "command_failed" | "javascript_error" | "unhandled_rejection" | "react_error";

/** Operational errors only: never include command arguments or React props. */
export function reportDiagnostic(kind: DiagnosticKind, error: unknown, details: { command?: string; source?: string; line?: number; column?: number } = {}) {
  if (!isTauri()) return;
  const message = error instanceof Error ? `${error.name}: ${error.message}` : typeof error === "string" ? error : "An operation failed";
  // Use invoke directly so an unavailable logger cannot recursively log itself.
  void invoke("report_frontend_event", { diagnostic: { kind, message: message.slice(0, 2000), ...details } }).catch(() => {});
}

export function installDiagnostics() {
  const onError = (event: ErrorEvent) => {
    reportDiagnostic("javascript_error", event.error ?? event.message, {
      source: event.filename, line: event.lineno, column: event.colno,
    });
  };
  const onRejection = (event: PromiseRejectionEvent) => reportDiagnostic("unhandled_rejection", event.reason);
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);
  return () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
  };
}
