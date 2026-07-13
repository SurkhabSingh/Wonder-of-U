import * as TabsPrimitive from "@radix-ui/react-tabs";

export type TranscriptLanguageTab = {
  code: string;
  label: string;
};

export function TranscriptLanguageTabs({
  value,
  tabs,
  onChange,
}: {
  value: string;
  tabs: TranscriptLanguageTab[];
  onChange: (code: string) => void;
}) {
  return (
    <TabsPrimitive.Root
      value={value}
      onValueChange={onChange}
      className="transcript-language-root"
    >
      <TabsPrimitive.List
        className="transcript-language-tabs"
        aria-label="Transcript languages"
      >
        {tabs.map((tab) => (
          <TabsPrimitive.Trigger
            key={tab.code}
            value={tab.code}
            className="transcript-language-tab"
          >
            {tab.label}
          </TabsPrimitive.Trigger>
        ))}
      </TabsPrimitive.List>
    </TabsPrimitive.Root>
  );
}
