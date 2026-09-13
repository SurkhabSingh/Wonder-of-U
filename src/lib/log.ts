import { invoke } from "@tauri-apps/api/core";

// Sends a line to the same file the Rust side writes.
 
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

//Routes the two ways the interface fails on its own into the log.

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
