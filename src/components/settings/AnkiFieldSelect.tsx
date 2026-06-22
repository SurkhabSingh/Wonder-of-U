import type { AnkiFieldMapping, SelectOption } from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";

export function AnkiFieldSelect({
  field,
  label,
  description,
  currentValue,
  fieldOptions,
  onChange,
}: {
  field: keyof AnkiFieldMapping;
  label: string;
  description?: string;
  currentValue: string;
  fieldOptions: string[];
  onChange: (field: keyof AnkiFieldMapping, value: string) => void;
}) {
  const options: SelectOption[] = [
    { value: "", label: "Not mapped" },
    ...(currentValue && !fieldOptions.includes(currentValue)
      ? [{ value: currentValue, label: currentValue }]
      : []),
    ...fieldOptions.map((fieldName) => ({
      value: fieldName,
      label: fieldName,
    })),
  ];

  return (
    <label className="field">
      <span className="field-label-with-help">
        <span>{label}</span>
        {description ? <TooltipBadge label="?" description={description} /> : null}
      </span>
      <ThemedSelect
        value={currentValue}
        options={options}
        placeholder="Not mapped"
        onChange={(nextValue) => onChange(field, nextValue)}
      />
    </label>
  );
}
