import type { ReactNode } from "react";
import {
  SCANNABLE_CLASS,
  SCANNABLE_END_ATTRIBUTE,
  SCANNABLE_OWNER_ATTRIBUTE,
  SCANNABLE_START_ATTRIBUTE,
} from "../../hooks/useWordScanner";

// Marks text whose words can be looked up.
export function ScannableText({
  ownerKey,
  startMs,
  endMs,
  children,
}: {
  ownerKey: string;
  startMs?: number | null;
  endMs?: number | null;
  children: ReactNode;
}) {
  const timed = typeof startMs === "number" && typeof endMs === "number";
  return (
    <span
      className={SCANNABLE_CLASS}
      {...{ [SCANNABLE_OWNER_ATTRIBUTE]: ownerKey }}
      {...(timed
        ? {
            [SCANNABLE_START_ATTRIBUTE]: String(startMs),
            [SCANNABLE_END_ATTRIBUTE]: String(endMs),
          }
        : {})}
    >
      {children}
    </span>
  );
}
