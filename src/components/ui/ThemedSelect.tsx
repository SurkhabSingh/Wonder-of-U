import * as SelectPrimitive from "@radix-ui/react-select";
import type { SelectOption } from "../../types";

const EMPTY_SELECT_VALUE = "__wonder_of_u_empty__";

function toRadixSelectValue(value: string): string {
  return value === "" ? EMPTY_SELECT_VALUE : value;
}

function fromRadixSelectValue(value: string): string {
  return value === EMPTY_SELECT_VALUE ? "" : value;
}

export function ThemedSelect({
  value,
  options,
  onChange,
  placeholder = "Select option",
  disabled = false,
  title,
  triggerClassName,
  describedBy,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  title?: string;
  // Id of an element describing this control. The trigger's `aria-label` replaces the
  // whole wrapping <label>, so anything else in that label — a badge, a caveat — is
  // dropped for screen readers unless it is pointed at explicitly.
  describedBy?: string;
  // Extra class on the trigger, so a caller can compact it (e.g. the player bar)
  // without forking the component or its styled dropdown.
  triggerClassName?: string;
}) {
  return (
    <SelectPrimitive.Root
      value={toRadixSelectValue(value)}
      onValueChange={(nextValue) => onChange(fromRadixSelectValue(nextValue))}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger
        className={`themed-select-trigger${triggerClassName ? ` ${triggerClassName}` : ""}`}
        title={title}
        aria-label={placeholder}
        aria-describedby={describedBy}
      >
        <SelectPrimitive.Value placeholder={placeholder} />
        <SelectPrimitive.Icon className="themed-select-icon" aria-hidden="true">
          {"\u25be"}
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          className="themed-select-content"
          position="popper"
          sideOffset={6}
        >
          <SelectPrimitive.Viewport className="themed-select-viewport">
            {options.map((option) => (
              <SelectPrimitive.Item
                key={`${option.value}:${option.label}`}
                className="themed-select-item"
                value={toRadixSelectValue(option.value)}
              >
                <SelectPrimitive.ItemText>{option.label}</SelectPrimitive.ItemText>
                <SelectPrimitive.ItemIndicator className="themed-select-check">
                  {"\u2713"}
                </SelectPrimitive.ItemIndicator>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}
