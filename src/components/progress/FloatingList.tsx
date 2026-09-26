import { useEffect, useRef, type ReactNode } from "react";

/** Opens over the page; shuts when the pointer or focus goes elsewhere, or on Escape. */
export function FloatingList({ label, children }: { label: string; children: ReactNode }) {
  const list = useRef<HTMLDetailsElement>(null);

  useEffect(() => {
    const inside = (target: EventTarget | null) =>
      target instanceof Node && list.current !== null && list.current.contains(target);
    const shut = (event: Event) => {
      if (list.current?.open && !inside(event.target)) {
        list.current.open = false;
      }
    };
    const escape = (event: KeyboardEvent) => {
      const focus = document.activeElement;
      if (event.key !== "Escape" || !list.current?.open) {
        return;
      }
      if (focus === document.body || inside(focus)) {
        list.current.open = false;
        list.current.querySelector("summary")?.focus();
      }
    };
    document.addEventListener("pointerdown", shut);
    document.addEventListener("focusin", shut);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", shut);
      document.removeEventListener("focusin", shut);
      document.removeEventListener("keydown", escape);
    };
  }, []);

  return (
    <details className="viz-table" ref={list}>
      <summary>{label}</summary>
      <div className="viz-table-scroll">{children}</div>
    </details>
  );
}
