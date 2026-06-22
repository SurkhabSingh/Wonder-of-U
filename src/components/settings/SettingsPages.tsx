import { LANGUAGE_OPTIONS, MODEL_OPTIONS } from "../../constants";
import { fileNameFromPath } from "../../lib/format";
import { whisperStatusLabel } from "../../lib/helpers";
import type {
  AnkiCatalog,
  AnkiFieldMapping,
  AnkiSettings,
  AppBootstrap,
  AppPage,
  AppSettings,
  AutosaveState,
  BusyAction,
  FeatureSettings,
  ThemePreference,
  WhisperAssetUpdateResult,
  WhisperSettings,
} from "../../types";
import { TooltipBadge } from "../ui/Tooltip";
import { ThemedSelect } from "../ui/ThemedSelect";
import { UpdateResultCard } from "../ui/UpdateResultCard";
import { AnkiFieldSelect } from "./AnkiFieldSelect";
import { DownloadProgressCard } from "./DownloadProgressCard";

type SettingsUpdate = Partial<Omit<AppSettings, "features" | "whisper" | "anki">> & {
  features?: Partial<FeatureSettings>;
  whisper?: Partial<WhisperSettings>;
  anki?: Partial<Omit<AnkiSettings, "fields">> & {
    fields?: Partial<AnkiFieldMapping>;
  };
};

export function SettingsPages({
  activePage,
  bootstrap,
  settingsDraft,
  autosaveState,
  autosaveMessage,
  busyAction,
  displayedAnkiCatalog,
  activeRuntimeVersion,
  installedRuntimeVersions,
  manualRuntimeOverride,
  runtimeUpdateResult,
  runtimeUpdateVersion,
  modelUpdateResult,
  runtimeInstalled,
  modelInstalled,
  resolvedCliPath,
  resolvedModelPath,
  downloadIsActive,
  onUpdateSettings,
  onBrowseDirectory,
  onBrowseFile,
  onCheckRuntimeUpdate,
  onDownloadRuntimeVersion,
  onDownloadRecommendedRuntime,
  onCheckModelUpdate,
  onDownloadRecommendedModel,
  onDownloadRecommendedFfmpeg,
  onToggleDownloadPause,
  onCancelDownload,
  onRefreshAnkiCatalog,
  onUpdateAnkiField,
}: {
  activePage: AppPage;
  bootstrap: AppBootstrap;
  settingsDraft: AppSettings;
  autosaveState: AutosaveState;
  autosaveMessage: string;
  busyAction: BusyAction;
  displayedAnkiCatalog: AnkiCatalog;
  activeRuntimeVersion: string;
  installedRuntimeVersions: string[];
  manualRuntimeOverride: boolean;
  runtimeUpdateResult: WhisperAssetUpdateResult | null;
  runtimeUpdateVersion: string | null;
  modelUpdateResult: WhisperAssetUpdateResult | null;
  runtimeInstalled: boolean;
  modelInstalled: boolean;
  resolvedCliPath: string;
  resolvedModelPath: string;
  downloadIsActive: boolean;
  onUpdateSettings: (update: SettingsUpdate) => void;
  onBrowseDirectory: (field: "outputDirectory" | "assetDirectory") => void | Promise<void>;
  onBrowseFile: (field: "cliPath" | "modelPath") => void | Promise<void>;
  onCheckRuntimeUpdate: () => void | Promise<void>;
  onDownloadRuntimeVersion: (version: string) => void | Promise<void>;
  onDownloadRecommendedRuntime: () => void | Promise<void>;
  onCheckModelUpdate: () => void | Promise<void>;
  onDownloadRecommendedModel: () => void | Promise<void>;
  onDownloadRecommendedFfmpeg: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  onCancelDownload: () => void | Promise<void>;
  onRefreshAnkiCatalog: (noteType?: string) => void | Promise<void>;
  onUpdateAnkiField: (field: keyof AnkiFieldMapping, value: string) => void;
}) {
  if (activePage === "recorder" || activePage === "recordings") {
    return null;
  }

  const selectedModel =
    MODEL_OPTIONS.find((option) => option.id === settingsDraft.whisper.modelChoice) ??
    MODEL_OPTIONS[2];
  const selectedLanguageCode = settingsDraft.whisper.language || "auto";
  const selectedLanguageKnown = LANGUAGE_OPTIONS.some(
    (option) => option.code === selectedLanguageCode,
  );

  return (
    <div className="settings-scroll settings-page-single">
      <div className="settings-overview-grid">
        <article
          className="panel settings-card"
          hidden={activePage !== "preferences"}
        >
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Settings</p>
              <h2>App Preferences</h2>
            </div>
          </header>

          {autosaveState === "error" ? (
            <p className="autosave-error" role="alert">
              {autosaveMessage}
            </p>
          ) : null}

          <div className="settings-grid">
            <label className="field">
              <span>Appearance</span>
              <ThemedSelect
                value={settingsDraft.theme}
                options={[
                  { value: "system", label: "Use system setting" },
                  { value: "light", label: "Light" },
                  { value: "dark", label: "Dark" },
                ]}
                placeholder="Appearance"
                onChange={(nextValue) =>
                  onUpdateSettings({
                    theme: nextValue as ThemePreference,
                  })
                }
              />
            </label>

            <label className="field">
              <span>Recording output folder</span>
              <div className="input-with-action">
                <input
                  type="text"
                  value={settingsDraft.outputDirectory}
                  onChange={(event) =>
                    onUpdateSettings({
                      outputDirectory: event.currentTarget.value,
                    })
                  }
                  placeholder="Choose where WAV files are stored"
                />
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void onBrowseDirectory("outputDirectory")}
                  disabled={busyAction === "browse"}
                >
                  Browse
                </button>
              </div>
            </label>

            <label className="field">
              <span>Model and asset folder</span>
              <div className="input-with-action">
                <input
                  type="text"
                  value={settingsDraft.assetDirectory}
                  onChange={(event) =>
                    onUpdateSettings({
                      assetDirectory: event.currentTarget.value,
                    })
                  }
                  placeholder="Choose where Whisper runtime and model assets live"
                />
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void onBrowseDirectory("assetDirectory")}
                  disabled={busyAction === "browse"}
                >
                  Browse
                </button>
              </div>
            </label>

            <div className="toggle-grid">
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settingsDraft.features.transcription}
                  onChange={(event) =>
                    onUpdateSettings({
                      features: {
                        transcription: event.currentTarget.checked,
                      },
                    })
                  }
                />
                <span>Enable transcription</span>
              </label>

              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settingsDraft.features.deleteLocalAudioAfterAnkiPush}
                  onChange={(event) => {
                    const enabled = event.currentTarget.checked;
                    if (enabled) {
                      const confirmed = window.confirm(
                        "Enable local audio cleanup after Anki push? After Anki successfully copies the audio into its media folder, Wonder of U will delete the local audio file from this machine. The transcript and history stay in Wonder of U, and existing Anki cards are not affected.",
                      );
                      if (!confirmed) {
                        return;
                      }
                    }
                    onUpdateSettings({
                      features: {
                        deleteLocalAudioAfterAnkiPush: enabled,
                      },
                    });
                  }}
                />
                <span>Delete local audio after Anki push</span>
              </label>

              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settingsDraft.features.allowMp3Conversion}
                  onChange={(event) =>
                    onUpdateSettings({
                      features: {
                        allowMp3Conversion: event.currentTarget.checked,
                      },
                    })
                  }
                />
                <span>Allow manual MP3 conversion</span>
              </label>

              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settingsDraft.launchAtLogin}
                  onChange={(event) =>
                    onUpdateSettings({
                      launchAtLogin: event.currentTarget.checked,
                    })
                  }
                />
                <span>Launch with Windows</span>
              </label>

              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settingsDraft.startMinimized}
                  onChange={(event) =>
                    onUpdateSettings({
                      startMinimized: event.currentTarget.checked,
                    })
                  }
                />
                <span>Start minimized to tray</span>
              </label>
            </div>
          </div>
        </article>

        <article className="panel settings-card" hidden={activePage !== "whisper"}>
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Whisper Setup</p>
              <h2>Whisper</h2>
            </div>
            <TooltipBadge
              label={whisperStatusLabel(bootstrap.whisperDetection.status)}
              description={bootstrap.whisperDetection.message}
            />
          </header>

          <div className="meta-list compact-meta-list">
            <div title={bootstrap.whisperDetection.executablePath || "Not installed"}>
              <span className="hint-label">Runtime</span>
              <strong>
                {bootstrap.whisperDetection.cliReady
                  ? `Ready (${activeRuntimeVersion})`
                  : "Missing"}
              </strong>
            </div>
            <div title={bootstrap.whisperDetection.modelPath || "Not installed"}>
              <span className="hint-label">Model</span>
              <strong>
                {bootstrap.whisperDetection.modelReady ? "Ready" : "Missing"}
              </strong>
            </div>
            <div>
              <span className="hint-label">Language</span>
              <strong>{settingsDraft.whisper.language}</strong>
            </div>
          </div>
        </article>
      </div>

      <div className="whisper-config-grid">
        <article className="panel settings-card" hidden={activePage !== "runtime"}>
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Runtime</p>
              <h2>Whisper CLI</h2>
            </div>
            <TooltipBadge
              label="?"
              description="Paste a path if whisper-cli is already installed somewhere else, or let the app download and manage the recommended Windows runtime."
            />
          </header>

          {installedRuntimeVersions.length > 0 ? (
            <label className="field runtime-version-field">
              <span>Active runtime</span>
              <ThemedSelect
                value={activeRuntimeVersion}
                options={installedRuntimeVersions.map((version) => ({
                  value: version,
                  label: version,
                }))}
                placeholder="Active runtime"
                onChange={(nextValue) =>
                  onUpdateSettings({
                    whisper: {
                      runtimeVersion: nextValue,
                      cliPath: "",
                    },
                  })
                }
                disabled={manualRuntimeOverride}
                title={
                  manualRuntimeOverride
                    ? "Clear the manual runtime override to use app-managed versions."
                    : "Choose any installed app-managed Whisper runtime."
                }
              />
            </label>
          ) : null}

          <details className="disclosure">
            <summary>Manual runtime override</summary>
            <label className="field">
              <span>whisper-cli path</span>
              <div className="input-with-action">
                <input
                  type="text"
                  value={resolvedCliPath}
                  onChange={(event) =>
                    onUpdateSettings({
                      whisper: {
                        cliPath: event.currentTarget.value,
                      },
                    })
                  }
                  placeholder="whisper-cli path"
                />
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void onBrowseFile("cliPath")}
                  disabled={busyAction === "browse"}
                >
                  Browse
                </button>
              </div>
            </label>
          </details>

          <div className="download-section">
            {runtimeInstalled ? (
              <div className="installed-card">
                <div className="installed-row">
                  <strong>Runtime ready</strong>
                  {bootstrap.whisperDetection.cliManaged ? (
                    <div className="action-row inline-actions">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => void onCheckRuntimeUpdate()}
                        disabled={busyAction === "checkRuntimeUpdate"}
                      >
                        Check for Updates
                      </button>
                    </div>
                  ) : null}
                </div>
                <UpdateResultCard result={runtimeUpdateResult} />
                {runtimeUpdateVersion ? (
                  <div className="action-row compact-actions">
                    <button
                      type="button"
                      onClick={() => void onDownloadRuntimeVersion(runtimeUpdateVersion)}
                      disabled={downloadIsActive || busyAction === "downloadRuntime"}
                    >
                      Download {runtimeUpdateVersion}
                    </button>
                  </div>
                ) : null}
              </div>
            ) : (
              <div className="action-row inline-actions">
                <button
                  type="button"
                  onClick={() => void onDownloadRecommendedRuntime()}
                  disabled={downloadIsActive || busyAction === "downloadRuntime"}
                >
                  Download Recommended Runtime
                </button>
              </div>
            )}
            <DownloadProgressCard
              snapshot={bootstrap.modelDownload}
              kind="runtime"
              downloadIsActive={downloadIsActive}
              onTogglePause={() => void onToggleDownloadPause()}
              onCancel={() => void onCancelDownload()}
            />
          </div>
        </article>

        <article className="panel settings-card" hidden={activePage !== "model"}>
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Model</p>
              <h2>Whisper Model</h2>
            </div>
            <TooltipBadge
              label="?"
              description="Choose a model file manually, or let the app download the recommended multilingual model into your selected asset folder."
            />
          </header>

          <div className="settings-grid">
            <label className="field">
              <span>Managed model</span>
              <ThemedSelect
                value={settingsDraft.whisper.modelChoice}
                options={MODEL_OPTIONS.map((option) => ({
                  value: option.id,
                  label: option.label,
                }))}
                placeholder="Managed model"
                onChange={(nextValue) =>
                  onUpdateSettings({
                    whisper: {
                      modelChoice: nextValue,
                    },
                  })
                }
              />
            </label>

            <label className="field">
              <span>Language</span>
              <ThemedSelect
                value={selectedLanguageCode}
                options={[
                  ...(!selectedLanguageKnown
                    ? [
                        {
                          value: selectedLanguageCode,
                          label: `Custom (${selectedLanguageCode})`,
                        },
                      ]
                    : []),
                  ...LANGUAGE_OPTIONS.map((option) => ({
                    value: option.code,
                    label: `${option.label} (${option.code})`,
                  })),
                ]}
                placeholder="Language"
                onChange={(nextValue) =>
                  onUpdateSettings({
                    whisper: {
                      language: nextValue,
                    },
                  })
                }
              />
            </label>
          </div>

          <div className="model-summary" title={selectedModel.description}>
            <strong>{selectedModel.label}</strong>
            <span>
              {selectedModel.diskSize} Â· {selectedModel.memoryUsage} RAM
            </span>
          </div>

          <details className="disclosure">
            <summary>Manual model override</summary>
            <label className="field">
              <span>GGML model path</span>
              <div className="input-with-action">
                <input
                  type="text"
                  value={resolvedModelPath}
                  onChange={(event) =>
                    onUpdateSettings({
                      whisper: {
                        modelPath: event.currentTarget.value,
                      },
                    })
                  }
                  placeholder="GGML model path"
                />
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void onBrowseFile("modelPath")}
                  disabled={busyAction === "browse"}
                >
                  Browse
                </button>
              </div>
            </label>
          </details>

          <div className="download-section">
            {modelInstalled ? (
              <div className="installed-card">
                <div className="installed-row">
                  <strong>Model ready</strong>
                  {bootstrap.whisperDetection.modelManaged ? (
                    <div className="action-row inline-actions">
                      <button
                        type="button"
                        className="secondary"
                        onClick={() => void onCheckModelUpdate()}
                        disabled={busyAction === "checkModelUpdate"}
                      >
                        Check for Updates
                      </button>
                    </div>
                  ) : null}
                </div>
                <UpdateResultCard result={modelUpdateResult} />
              </div>
            ) : (
              <div className="action-row inline-actions">
                <button
                  type="button"
                  className="secondary"
                  onClick={() => void onDownloadRecommendedModel()}
                  disabled={downloadIsActive || busyAction === "downloadModel"}
                >
                  Download {selectedModel.label} Model
                </button>
              </div>
            )}
            <DownloadProgressCard
              snapshot={bootstrap.modelDownload}
              kind="model"
              downloadIsActive={downloadIsActive}
              onTogglePause={() => void onToggleDownloadPause()}
              onCancel={() => void onCancelDownload()}
            />
          </div>
        </article>

        <article
          className="panel settings-card settings-card-wide"
          hidden={activePage !== "storage"}
        >
          <header className="panel-header">
            <div>
              <p className="panel-kicker">Storage</p>
              <h2>MP3 Compression</h2>
            </div>
            <TooltipBadge
              label={bootstrap.ffmpegDetection.status === "ready" ? "Ready" : "Missing"}
              description={bootstrap.ffmpegDetection.message}
            />
          </header>

          <div
            className={`update-card ${
              bootstrap.ffmpegDetection.status === "ready" ? "current" : "available"
            }`}
          >
            <strong>{bootstrap.ffmpegDetection.message}</strong>
            <p className="microcopy">
              Wonder of U keeps WAV audio for transcription because that is the
              safest input path for Whisper. After a transcript exists, you can
              convert individual recordings, selected recordings, or all available
              WAV recordings to MP3 from Saved Recordings. If a card was already
              pushed to Anki, converting the local file later will not break that
              existing Anki card because Anki keeps its own copied media file. The
              Convert to MP3 action stays hidden until you enable manual MP3
              conversion in App Preferences.
            </p>
            {bootstrap.ffmpegDetection.executablePath ? (
              <p
                className="path-copy"
                title={bootstrap.ffmpegDetection.executablePath}
              >
                {fileNameFromPath(bootstrap.ffmpegDetection.executablePath)}
              </p>
            ) : null}
          </div>

          {bootstrap.ffmpegDetection.status !== "ready" ? (
            <div className="action-row inline-actions">
              <button
                type="button"
                className="secondary"
                onClick={() => void onDownloadRecommendedFfmpeg()}
                disabled={downloadIsActive || busyAction === "downloadFfmpeg"}
              >
                Download FFmpeg
              </button>
            </div>
          ) : null}
          <DownloadProgressCard
            snapshot={bootstrap.modelDownload}
            kind="ffmpeg"
            downloadIsActive={downloadIsActive}
            onTogglePause={() => void onToggleDownloadPause()}
            onCancel={() => void onCancelDownload()}
          />
        </article>
      </div>

      <article
        className="panel anki-panel settings-card settings-card-wide"
        hidden={activePage !== "anki"}
      >
        <header className="panel-header">
          <div>
            <p className="panel-kicker">Anki</p>
            <h2>Card Mapping</h2>
          </div>
          <div className="panel-actions">
            <TooltipBadge
              label={displayedAnkiCatalog.status === "ready" ? "Ready" : "Saved"}
              description={displayedAnkiCatalog.message}
            />
            <button
              type="button"
              className="secondary"
              onClick={() => void onRefreshAnkiCatalog()}
              disabled={busyAction === "loadAnki"}
            >
              Refresh Anki
            </button>
          </div>
        </header>

        <div
          className={`update-card ${
            displayedAnkiCatalog.status === "ready"
              ? "current"
              : displayedAnkiCatalog.status === "offline"
                ? "error"
                : ""
          }`}
        >
          <strong>{displayedAnkiCatalog.message}</strong>
          {displayedAnkiCatalog.version !== null ? (
            <p className="microcopy">
              AnkiConnect version {displayedAnkiCatalog.version}
            </p>
          ) : null}
        </div>

        <div className="settings-grid anki-grid">
          <label className="field">
            <span className="field-label-with-help">
              <span>Deck</span>
              <TooltipBadge
                label="?"
                description="Cards are created in this Anki deck when you use the default Push action. Push to another deck overrides this only for that action."
              />
            </span>
            <ThemedSelect
              value={settingsDraft.anki.deckName}
              options={[
                { value: "", label: "Choose deck" },
                ...(settingsDraft.anki.deckName &&
                !displayedAnkiCatalog.decks.includes(settingsDraft.anki.deckName)
                  ? [
                      {
                        value: settingsDraft.anki.deckName,
                        label: settingsDraft.anki.deckName,
                      },
                    ]
                  : []),
                ...displayedAnkiCatalog.decks.map((deck) => ({
                  value: deck,
                  label: deck,
                })),
              ]}
              placeholder="Choose deck"
              onChange={(nextValue) =>
                onUpdateSettings({
                  anki: {
                    deckName: nextValue,
                  },
                })
              }
            />
          </label>

          <label className="field">
            <span className="field-label-with-help">
              <span>Note type</span>
              <TooltipBadge
                label="?"
                description="This controls which Anki fields are available below. If you change the note type, the field mapping is reset because each note type has different fields."
              />
            </span>
            <ThemedSelect
              value={settingsDraft.anki.noteType}
              options={[
                { value: "", label: "Choose note type" },
                ...(settingsDraft.anki.noteType &&
                !displayedAnkiCatalog.noteTypes.includes(settingsDraft.anki.noteType)
                  ? [
                      {
                        value: settingsDraft.anki.noteType,
                        label: settingsDraft.anki.noteType,
                      },
                    ]
                  : []),
                ...displayedAnkiCatalog.noteTypes.map((noteType) => ({
                  value: noteType,
                  label: noteType,
                })),
              ]}
              placeholder="Choose note type"
              onChange={(noteType) => {
                onUpdateSettings({
                  anki: {
                    noteType,
                    fields: {
                      transcription: "",
                      furigana: "",
                      audio: "",
                      translation: "",
                      sourcePath: "",
                      createdAt: "",
                    },
                  },
                });
                if (noteType) {
                  void onRefreshAnkiCatalog(noteType);
                }
              }}
            />
          </label>

          <AnkiFieldSelect
            field="transcription"
            label="Expression / transcript field"
            description="Receives the transcript during push. When furigana is enabled or added later, this same field is replaced with hover-only ruby HTML, like a Yomitan expression field."
            currentValue={settingsDraft.anki.fields.transcription}
            fieldOptions={displayedAnkiCatalog.fields}
            onChange={onUpdateAnkiField}
          />
          <AnkiFieldSelect
            field="audio"
            label="Replay audio field"
            description="Receives the [sound:...] tag. The replay icon only appears on card sides that render this field. If it disappears after revealing the answer, the Back template must include the front side or this audio field."
            currentValue={settingsDraft.anki.fields.audio}
            fieldOptions={displayedAnkiCatalog.fields}
            onChange={onUpdateAnkiField}
          />
          <AnkiFieldSelect
            field="translation"
            label="Translation field"
            description="Optional translated text. Leave unmapped if you do not want translations written to Anki."
            currentValue={settingsDraft.anki.fields.translation}
            fieldOptions={displayedAnkiCatalog.fields}
            onChange={onUpdateAnkiField}
          />
          <AnkiFieldSelect
            field="sourcePath"
            label="Source path field"
            description="Optional local audio path for your own tracking. This is not required for playback after Anki copies the media."
            currentValue={settingsDraft.anki.fields.sourcePath}
            fieldOptions={displayedAnkiCatalog.fields}
            onChange={onUpdateAnkiField}
          />
          <AnkiFieldSelect
            field="createdAt"
            label="Created-at field"
            description="Optional recording timestamp in milliseconds. Leave unmapped unless your note type has a tracking field for it."
            currentValue={settingsDraft.anki.fields.createdAt}
            fieldOptions={displayedAnkiCatalog.fields}
            onChange={onUpdateAnkiField}
          />
        </div>

        <div className="update-card">
          <label className="toggle inline-toggle">
            <input
              type="checkbox"
              checked={settingsDraft.features.autoAddFuriganaAfterAnkiPush}
              onChange={(event) =>
                onUpdateSettings({
                  features: {
                    autoAddFuriganaAfterAnkiPush: event.currentTarget.checked,
                  },
                })
              }
            />
            <span>Automatically add furigana when pushing Japanese cards</span>
          </label>
          <p className="microcopy">
            Requires the Wonder of U Anki add-on to be running. If the add-on is
            unavailable, Wonder of U still pushes the card and warns that furigana
            was skipped. Furigana is written onto the expression/transcript field
            itself.
          </p>
        </div>

        <div className="update-card">
          <strong>
            Recommended mapping: Expression / transcript -&gt; Expression or Back,
            Replay audio -&gt; Audio or Front.
          </strong>
          <p className="microcopy">
            Furigana is applied directly to the expression/transcript field, not a
            separate field. The Anki replay icon only shows if the audio field is
            visible in the current card side template.
          </p>
        </div>
      </article>
    </div>
  );
}
