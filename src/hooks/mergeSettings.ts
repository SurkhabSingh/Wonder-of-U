import type { DeepPartial } from "../types";

// Merge a partial settings update into the current settings.

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.getPrototypeOf(value) === Object.prototype
  );
}

function mergeRecords(
  current: Record<string, unknown>,
  update: Record<string, unknown>,
): Record<string, unknown> {
  const next = { ...current };

  for (const [key, incoming] of Object.entries(update)) {
    // An explicit undefined means "no opinion", not "clear it" — a caller building an
    // update object conditionally would otherwise erase a field by leaving a key unset.
    if (incoming === undefined) {
      continue;
    }

    const existing = current[key];
    next[key] =
      isPlainObject(existing) && isPlainObject(incoming)
        ? mergeRecords(existing, incoming)
        : incoming;
  }

  return next;
}

export function mergeSettings<T extends object>(
  current: T,
  update: DeepPartial<T>,
): T {
  return mergeRecords(
    current as Record<string, unknown>,
    update as Record<string, unknown>,
  ) as T;
}
