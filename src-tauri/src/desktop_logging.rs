use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticKind {
    CommandFailed,
    JavascriptError,
    UnhandledRejection,
    ReactError,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrontendDiagnostic {
    kind: DiagnosticKind,
    message: String,
    command: Option<String>,
    source: Option<String>,
    line: Option<u32>,
    column: Option<u32>,
}

#[tauri::command]
pub(crate) fn report_frontend_event(window: tauri::WebviewWindow, diagnostic: FrontendDiagnostic) {
    let (level, event, message) = match diagnostic.kind {
        DiagnosticKind::CommandFailed => (
            "warn",
            "frontend_command_failed",
            "A desktop command failed",
        ),
        DiagnosticKind::JavascriptError => ("error", "frontend_error", "A frontend error occurred"),
        DiagnosticKind::UnhandledRejection => (
            "error",
            "frontend_unhandled_rejection",
            "A frontend promise was rejected",
        ),
        DiagnosticKind::ReactError => (
            "error",
            "frontend_react_error",
            "A React render error occurred",
        ),
    };
    crate::remote_logging::event(
        level,
        event,
        message,
        json!({
            "window": window.label(),
            "error": diagnostic.message,
            "command": diagnostic.command,
            "source": diagnostic.source,
            "line": diagnostic.line,
            "column": diagnostic.column,
        }),
    );
}
