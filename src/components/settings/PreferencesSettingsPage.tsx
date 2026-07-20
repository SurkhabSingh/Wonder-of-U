import { useConfirm } from "../ui/ConfirmDialogProvider";
import type {
  AppSettings,
  AutosaveState,
  BusyAction,
  IndicatorPosition,
  ThemePreference,
} from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import type { BrowseDirectoryField, SettingsUpdate } from "./settingsTypes";

export function PreferencesSettingsPage({
  autosaveMessage,
  autosaveState,
  busyAction,
  onBrowseDirectory,
  onUpdateSettings,
  settingsDraft,
}: {
  autosaveMessage: string;
  autosaveState: AutosaveState;
  busyAction: BusyAction;
  onBrowseDirectory: (field: BrowseDirectoryField) => void | Promise<void>;
  onUpdateSettings: (update: SettingsUpdate) => void;
  settingsDraft: AppSettings;
}) {
  const confirm = useConfirm();

  return (
    <>
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
          <span>Recording indicator position</span>
          <ThemedSelect
            value={settingsDraft.indicatorPosition}
            options={[
              { value: "top-left", label: "Top left" },
              { value: "top-center", label: "Top center" },
              { value: "top-right", label: "Top right" },
              { value: "bottom-left", label: "Bottom left" },
              { value: "bottom-center", label: "Bottom center" },
              { value: "bottom-right", label: "Bottom right" },
            ]}
            placeholder="Indicator position"
            onChange={(nextValue) =>
              onUpdateSettings({
                indicatorPosition: nextValue as IndicatorPosition,
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
              checked={settingsDraft.features.translateAfterTranscription}
              onChange={(event) =>
                onUpdateSettings({
                  features: {
                    translateAfterTranscription: event.currentTarget.checked,
                  },
                })
              }
            />
            <span>Translate after transcription</span>
          </label>

          <label className="toggle">
            <input
              type="checkbox"
              checked={settingsDraft.features.deleteLocalAudioAfterAnkiPush}
              onChange={async (event) => {
                const enabled = event.currentTarget.checked;
                if (enabled) {
                  const confirmed = await confirm({
                    title: "Enable audio cleanup?",
                    message:
                      "Enable local audio cleanup after Anki push? After Anki successfully copies the audio into its media folder, Wonder of U will delete the local audio file from this machine. The transcript and history stay in Wonder of U, and existing Anki cards are not affected.",
                    okLabel: "Enable",
                    cancelLabel: "Cancel",
                    danger: true,
                  });
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

        {settingsDraft.features.translateAfterTranscription ? (
          <p className="microcopy">
            Translation runs in the Wonder of U browser extension, so keep it
            open in App Support mode. The browser window can stay minimized. If
            it is not connected, the transcript is still saved and the
            translation is skipped.
          </p>
        ) : null}
      </div>
    </>
  );
}
