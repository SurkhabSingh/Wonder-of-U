import { toast } from "sonner";
import {
  DEEPL_TARGET_LANGUAGE_OPTIONS,
  DEFAULT_TRANSLATION_TARGET_LANGUAGE,
  GOOGLE_TARGET_LANGUAGE_OPTIONS,
} from "../../constants";
import type {
  AppSettings,
  LanguageOption,
  TranslationProvider,
} from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import type { SettingsUpdate } from "./settingsTypes";

const PROVIDER_OPTIONS: { value: TranslationProvider; label: string }[] = [
  { value: "google-translate", label: "Google Translate" },
  { value: "deepl", label: "DeepL" },
];

function targetLanguagesFor(provider: TranslationProvider): readonly LanguageOption[] {
  return provider === "deepl"
    ? DEEPL_TARGET_LANGUAGE_OPTIONS
    : GOOGLE_TARGET_LANGUAGE_OPTIONS;
}

function labelFor(
  options: readonly LanguageOption[],
  code: string,
): string | null {
  return options.find((option) => option.code === code)?.label ?? null;
}

export function TranslationSettingsPage({
  onUpdateSettings,
  settingsDraft,
}: {
  onUpdateSettings: (update: SettingsUpdate) => void;
  settingsDraft: AppSettings;
}) {
  const { provider, targetLanguage } = settingsDraft.translation;
  const targetLanguages = targetLanguagesFor(provider);
  const targetLanguageKnown = targetLanguages.some(
    (option) => option.code === targetLanguage,
  );

  function handleProviderChange(nextProvider: TranslationProvider) {
    const nextLanguages = targetLanguagesFor(nextProvider);
    const stillSupported = nextLanguages.some(
      (option) => option.code === targetLanguage,
    );
    const providerLabel = PROVIDER_OPTIONS.find(
      (option) => option.value === nextProvider,
    )?.label;

    if (!stillSupported) {
      const strandedLabel = labelFor(targetLanguages, targetLanguage);
      toast.warning(
        `${providerLabel} cannot translate into ${strandedLabel ?? targetLanguage}. Target language switched to English.`,
        { duration: 5000 },
      );
    }

    onUpdateSettings({
      translation: {
        provider: nextProvider,
        ...(stillSupported
          ? {}
          : { targetLanguage: DEFAULT_TRANSLATION_TARGET_LANGUAGE }),
      },
    });
  }

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
            value={provider}
            options={PROVIDER_OPTIONS}
            placeholder="Translation provider"
            onChange={(nextValue) =>
              handleProviderChange(nextValue as TranslationProvider)
            }
          />
        </label>

        <label className="field">
          <span>Target language</span>
          <ThemedSelect
            value={targetLanguage}
            options={[
              ...(!targetLanguageKnown
                ? [
                    {
                      value: targetLanguage,
                      label: `Custom (${targetLanguage})`,
                    },
                  ]
                : []),
              ...targetLanguages.map((option) => ({
                value: option.code,
                label: `${option.label} (${option.code})`,
              })),
            ]}
            placeholder="Target language"
            onChange={(nextValue) =>
              onUpdateSettings({
                translation: {
                  targetLanguage: nextValue,
                },
              })
            }
          />
        </label>

        <p className="microcopy">
          Transcripts are translated into this language. DeepL offers far fewer
          languages than Google Translate, so this list changes with the
          provider.
        </p>

        <p className="microcopy">
          Translations run in the Wonder of U browser extension. This tells it
          which service to use — DeepL falls back to page automation unless you
          set a DeepL API key in the extension. Keep the extension open in App
          Support mode while translating.
        </p>

        <div className="info-note">
          <p className="microcopy">
            <strong>Mining one line at a time:</strong> a mined card gets its own
            translation only when the service returns one line for each line sent.
            DeepL with an API key does. Google Translate translates the transcript
            as a whole, so lines can be merged or split — when they no longer line
            up, the card is mined without a translation and says so.
          </p>
        </div>
      </div>
    </>
  );
}
