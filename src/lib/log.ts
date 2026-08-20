import { invoke } from "@tauri-apps/api/core";

/**
 * Sends a line to the same file the Rust side writes.
 *
 * Nothing from the interface reached the log before this, so a report from a user was silent
 * about half the app: a failed command, a render that threw, a rejected promise. The backend
 * owns the file, the redaction and the rotation; this only hands it a record.
 *
 * Failures here are swallowed deliberately — a logger that throws while reporting a throw turns
 * one problem into two, and the console still has it during development.
 */
export function logToFile(
  level: "INFO" | "WARN" | "ERROR",
  event: string,
  message: string,
  details?: Record<string, unknown>,
): void {
  void invoke("log_from_ui", {
    level,
    event,
    message,
    details: details ?? {},
  }).catch((error) => {
    console.error("could not write to the log", error);
  });
}

/** What an unknown thrown value can be described as. */
function describe(value: unknown): { message: string; stack?: string } {
  if (value instanceof Error) {
    return { message: `${value.name}: ${value.message}`, stack: value.stack };
  }
  if (typeof value === "string") {
    return { message: value };
  }
  try {
    return { message: JSON.stringify(value) };
  } catch {
    return { message: String(value) };
  }
}

/**
 * Routes the two ways the interface fails on its own into the log.
 *
 * `error` covers anything thrown outside React's control — an event handler, a timer, a
 * callback. A render that throws is caught by the boundary instead and never reaches here.
 * `unhandledrejection` is where a rejected `invoke` lands when nothing caught it. Neither
 * leaves any trace otherwise, because a packaged build has no console anyone will read.
 *
 * The listeners live for the life of the process, so there is nothing to remove.
 */
export function installGlobalErrorLogging(): void {
  const onError = (event: ErrorEvent) => {
    const described = describe(event.error ?? event.message);
    logToFile("ERROR", "ui.error", described.message, {
      source: event.filename,
      line: event.lineno,
      column: event.colno,
      stack: described.stack,
    });
  };

  const onRejection = (event: PromiseRejectionEvent) => {
    const described = describe(event.reason);
    logToFile("ERROR", "ui.unhandled_rejection", described.message, {
      stack: described.stack,
    });
  };

  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);
}
