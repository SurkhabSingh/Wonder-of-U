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
