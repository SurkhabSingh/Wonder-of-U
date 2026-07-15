import type { ReactElement } from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";

export function TooltipBadge({
  label,
  description,
}: {
  label: string;
  description: string;
}) {
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger asChild>
        <span className="tooltip-badge" tabIndex={0} aria-label={description}>
          {label}
        </span>
      </TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content className="themed-tooltip-content" sideOffset={7}>
          {description}
          <TooltipPrimitive.Arrow className="themed-tooltip-arrow" />
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}

export function TooltipWrap({
  description,
  children,
}: {
  description: string;
  children: ReactElement;
}) {
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content className="themed-tooltip-content" sideOffset={7}>
          {description}
          <TooltipPrimitive.Arrow className="themed-tooltip-arrow" />
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}
