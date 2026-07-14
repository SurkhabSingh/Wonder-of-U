import * as TabsPrimitive from "@radix-ui/react-tabs";
import type { RecordingFilterTab } from "../../lib/navigation";
import type { RecordingFilter } from "../../types";

export function RecordingFilterTabs({
  value,
  tabs,
  onChange,
}: {
  value: RecordingFilter;
  tabs: RecordingFilterTab[];
  onChange: (filter: RecordingFilter) => void;
}) {
  return (
    <TabsPrimitive.Root
      value={value}
      onValueChange={(nextValue) => onChange(nextValue as RecordingFilter)}
      className="recording-filter-root"
    >
      <TabsPrimitive.List
        className="recording-filter-tabs"
        aria-label="Saved recording filters"
      >
        {tabs.map((tab) => (
          <TabsPrimitive.Trigger
            key={tab.id}
            value={tab.id}
            className="recording-filter-tab"
            data-filter={tab.id}
          >
            <span>{tab.label}</span>
            <strong className="status-chip-count">{tab.count}</strong>
          </TabsPrimitive.Trigger>
        ))}
      </TabsPrimitive.List>
    </TabsPrimitive.Root>
  );
}
