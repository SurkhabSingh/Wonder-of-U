import type {
  AnkiFieldMapping,
  AnkiSettings,
  AppSettings,
  FeatureSettings,
  WhisperSettings,
} from "../../types";

export type SettingsUpdate = Partial<
  Omit<AppSettings, "features" | "whisper" | "anki">
> & {
  features?: Partial<FeatureSettings>;
  whisper?: Partial<WhisperSettings>;
  anki?: Partial<Omit<AnkiSettings, "fields">> & {
    fields?: Partial<AnkiFieldMapping>;
  };
};

export type BrowseDirectoryField = "outputDirectory" | "assetDirectory";
export type BrowseFileField = "cliPath" | "modelPath";
