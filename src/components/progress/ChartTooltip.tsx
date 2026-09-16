import { useCallback, useRef, useState, type FocusEvent, type PointerEvent } from "react";

export type ChartTip = { x: number; value: string; label: string; y: number };

// Delegated: a mark carries its reading in data attributes, so a year of cells needs no
// handler of its own.
export function useChartTooltip() {
  const frame = useRef<HTMLDivElement | null>(null);
  const [tip, setTip] = useState<ChartTip | null>(null);

  const show = useCallback((target: EventTarget | null) => {
    const mark =
      target instanceof Element ? target.closest<HTMLElement>("[data-tip-value]") : null;
    const box = frame.current;
    if (!mark || !box) {
      setTip(null);
      return;
    }
    const at = mark.getBoundingClientRect();
    const within = box.getBoundingClientRect();
    const centre = at.left + at.width / 2 - within.left;
    setTip({
      x: Math.min(Math.max(centre, 64), within.width - 64),
      y: at.top - within.top,
      value: mark.dataset.tipValue ?? "",
      label: mark.dataset.tipLabel ?? "",
    });
  }, []);

  const hide = useCallback(() => setTip(null), []);

  return {
    frame,
    tip,
    handlers: {
      onPointerOver: (event: PointerEvent) => show(event.target),
      onPointerLeave: hide,
      onFocus: (event: FocusEvent) => show(event.target),
      onBlur: hide,
    },
  };
}

export function ChartTooltip({ tip }: { tip: ChartTip | null }) {
  if (!tip) {
    return null;
  }
  return (
    <div className="viz-tip" style={{ left: tip.x, top: tip.y }} aria-hidden="true">
      <strong>{tip.value}</strong>
      <span>{tip.label}</span>
    </div>
  );
}
