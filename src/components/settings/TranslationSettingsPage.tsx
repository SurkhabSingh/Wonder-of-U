import type { AppSettings, TranslationProvider } from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import type { SettingsUpdate } from "./settingsTypes";

// Must match the extension's provider ids (KNOWN_TRANSLATION_PROVIDERS). The
// desktop app sends the chosen id with every translation job; the extension
// routes on it instead of falling back to its own popup selection.
const PROVIDER_OPTIONS: { value: TranslationProvider; label: string }[] = [
  { value: "google-translate", label: "Google Translate" },
  { value: "deepl", label: "DeepL" },
];

export function TranslationSettingsPage({
  onUpdateSettings,
  settingsDraft,
}: {
  onUpdateSettings: (update: SettingsUpdate) => void;
  settingsDraft: AppSettings;
}) {
  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Settings</p>
          <h2>Translation</h2>
        </div>
      </header>

      <div className="settings-grid">
        <label className="field">
          <span>Translation provider</span>
          <ThemedSelect
            value={settingsDraft.translation.provider}
            options={PROVIDER_OPTIONS}
            placeholder="Translation provider"
            onChange={(nextValue) =>
              onUpdateSettings({
                translation: {
                  provider: nextValue as TranslationProvider,
                },
              })
            }
          />
        </label>

        <p className="microcopy">
          Translations run in the Wonder of U browser extension. This tells it
          which service to use — DeepL falls back to page automation unless you
          set a DeepL API key in the extension. Keep the extension open in App
          Support mode while translating.
        </p>
      </div>
    </>
  );
}
