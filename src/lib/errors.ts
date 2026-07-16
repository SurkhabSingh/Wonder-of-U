// Tauri rejects `invoke()` with the plain string from the Rust `Err(String)` —
// never an `Error` instance. An `error instanceof Error` check therefore always
// misses and every backend reason ("This is a live or upcoming stream…") gets
// replaced by a generic fallback. Read the string first, then the Error, then a
// `{ message }` carrier (the dialog/event plugins and the JS runtime both throw
// real Errors, so all three shapes reach this).
//
// The fallback wins only when nothing usable is left, blank strings included: a
// toast or banner reading "" is worse than the generic sentence.
export function errorMessage(error: unknown, fallback: string): string {
  const extracted = extractMessage(error)?.trim() ?? "";
  return extracted.length > 0 ? extracted : fallback;
}

function extractMessage(error: unknown): string | null {
  if (typeof error === "string") {
    return error;
  }

  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "object" && error !== null && "message" in error) {
    const { message } = error;
    return typeof message === "string" ? message : null;
  }

  return null;
}
