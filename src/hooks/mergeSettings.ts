import type { DeepPartial } from "../types";

/**
 * Merge a partial settings update into the current settings.
 *
 * This replaces a hand-written merge that spread each nested group by name, under a
 * comment warning that a group without its own line would be REPLACED wholesale by
 * whatever partial the caller passed — silently wiping its siblings, with no type
 * error to catch it. All five groups did have a line, so nothing was broken; the
 * problem was that staying unbroken depended on reading the comment. Walking the
 * shape instead means a sixth group is merged correctly the day it is added.
 *
 * Only plain objects recurse. Anything else — an array, a string, a number — is a
 * value the update means to replace. That matters most for the array of vocabulary
 * sources: merging it by index would make removing a row impossible, since the row
 * being dropped would simply survive from the current value. `DeepPartial` marks
 * arrays as leaves for the same reason, so the type says what this does.
 */
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

/**
 * The recursion runs untyped over plain records and the types are asserted once, here,
 * rather than cast at every level. `DeepPartial<T>[K]` and `DeepPartial<T[K]>` are the
 * same type by the mapped type's own definition, but TypeScript cannot see that through
 * a generic indexed access, and a cast per level would bury the one place worth checking.
 */
export function mergeSettings<T extends object>(current: T, update: DeepPartial<T>): T {
  return mergeRecords(
    current as Record<string, unknown>,
    update as Record<string, unknown>,
  ) as T;
}
