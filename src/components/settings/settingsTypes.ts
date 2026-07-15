import type {
  AnkiFieldMapping,
  AnkiSettings,
  AppSettings,
  FeatureSettings,
  TranslationSettings,
  WhisperSettings,
} from "../../types";

export type SettingsUpdate = Partial<
  Omit<AppSettings, "features" | "whisper" | "anki" | "translation">
> & {
  features?: Partial<FeatureSettings>;
  whisper?: Partial<WhisperSettings>;
  translation?: Partial<TranslationSettings>;
  anki?: Partial<Omit<AnkiSettings, "fields">> & {
    fields?: Partial<AnkiFieldMapping>;
  };
};

export type BrowseDirectoryField = "outputDirectory" | "assetDirectory";
export type BrowseFileField = "cliPath" | "modelPath";
