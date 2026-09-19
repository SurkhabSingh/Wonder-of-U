import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type PointerEvent,
} from "react";

// `x` is the mark's centre and `bound` the chart's width, both from the chart's left edge.
export type ChartTip = { x: number; value: string; label: string; y: number; bound: number };

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
    setTip({
      x: at.left + at.width / 2 - within.left,
      y: at.top - within.top,
      bound: within.width,
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
  const box = useRef<HTMLDivElement | null>(null);

  // Placed once its width is known and before it is painted, so a long label stays inside
  // the chart rather than running past its edge.
  useLayoutEffect(() => {
    const element = box.current;
    if (!element || !tip) {
      return;
    }
    const width = element.offsetWidth;
    const left = Math.min(Math.max(tip.x - width / 2, 0), Math.max(tip.bound - width, 0));
    element.style.left = `${left}px`;
  }, [tip]);

  if (!tip) {
    return null;
  }
  return (
    <div ref={box} className="viz-tip" style={{ top: tip.y }} aria-hidden="true">
      <strong>{tip.value}</strong>
      <span>{tip.label}</span>
    </div>
  );
}
