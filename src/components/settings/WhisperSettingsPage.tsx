import { LANGUAGE_OPTIONS, MODEL_OPTIONS } from "../../constants";
import { whisperStatusLabel } from "../../lib/helpers";
import type {
  AppBootstrap,
  AppSettings,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import { DownloadProgressCard } from "./DownloadProgressCard";
import type { BrowseFileField, SettingsUpdate } from "./settingsTypes";
import { SettingsDisclosure } from "./SettingsDisclosure";

function whisperStatusTone(status: string): "success" | "warning" | "error" {
  if (status === "ready") {
    return "success";
  }
  if (status === "invalid") {
    return "error";
  }
  return "warning";
}

// Everything Whisper needs, in one section.
//
// This was three cards — a read-only status card, a runtime card and a model card — and they
// spent most of their space repeating each other. The status card showed the active runtime
// version and the language as read-only rows directly above the controls that set those same
// two values, and both of the others carried a manual-path override built from identical
// markup. What is left is the two things that are genuinely separate: a runtime and a model,
// each its own download with its own version, path and update check.
//
// The status rows are gone rather than moved. A value shown next to the control that edits it
// is not status, it is the control's value said twice.
export function WhisperSettingsPage({
  activeRuntimeVersion,
  bootstrap,
  busyAction,
  downloadIsActive,
  installedRuntimeVersions,
  modelInstalled,
  modelUpdateResult,
  onBrowseFile,
  onCancelDownload,
  onCheckModelUpdate,
  onCheckRuntimeUpdate,
  onDownloadRecommendedModel,
  onDownloadRecommendedRuntime,
  onDownloadRuntimeVersion,
  onToggleDownloadPause,
  onUpdateSettings,
  resolvedCliPath,
  resolvedModelPath,
  runtimeInstalled,
  runtimeUpdateResult,
  runtimeUpdateVersion,
  settingsDraft,
}: {
  activeRuntimeVersion: string;
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  downloadIsActive: boolean;
  installedRuntimeVersions: string[];
  modelInstalled: boolean;
  modelUpdateResult: WhisperAssetUpdateResult | null;
  onBrowseFile: (field: BrowseFileField) => void | Promise<void>;
  onCancelDownload: () => void | Promise<void>;
  onCheckModelUpdate: () => void | Promise<void>;
  onCheckRuntimeUpdate: () => void | Promise<void>;
  onDownloadRecommendedModel: () => void | Promise<void>;
  onDownloadRecommendedRuntime: () => void | Promise<void>;
  onDownloadRuntimeVersion: (version: string) => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  onUpdateSettings: (update: SettingsUpdate) => void;
  resolvedCliPath: string;
  resolvedModelPath: string;
  runtimeInstalled: boolean;
  runtimeUpdateResult: WhisperAssetUpdateResult | null;
  runtimeUpdateVersion: string | null;
  settingsDraft: AppSettings;
}) {
  // Both overrides are the same question asked of two settings. One of them used to be
  // computed three components upstream and threaded down; there was never a second consumer.
  const manualRuntimeOverride = settingsDraft.whisper.cliPath.trim().length > 0;
  const manualModelOverride = settingsDraft.whisper.modelPath.trim().length > 0;

  const selectedModel =
    MODEL_OPTIONS.find((option) => option.id === settingsDraft.whisper.modelChoice) ??
    MODEL_OPTIONS[2];
  const selectedLanguageCode = settingsDraft.whisper.language || "auto";
  const selectedLanguageKnown = LANGUAGE_OPTIONS.some(
    (option) => option.code === selectedLanguageCode,
  );

  // The model check compares the local file against the one at the model's fixed URL, so
  // re-downloading the recommended model IS the install for whatever it found.
  const modelUpdateAvailable = modelUpdateResult?.status === "available";
  const whisperStatus = bootstrap.whisperDetection.status;

  return (
    <SettingsDisclosure
      title="Whisper"
      defaultOpen={!runtimeInstalled || !modelInstalled}
      tone={
        whisperStatus === "ready"
          ? "ready"
          : whisperStatus === "invalid"
            ? "error"
            : "attention"
      }
      status={
        <span
          className={`status-chip status-chip-${whisperStatusTone(whisperStatus)}`}
        >
          {whisperStatusLabel(whisperStatus)}
        </span>
      }
    >
      <section className="settings-group">
        <header className="settings-group-header">
          <h3>Runtime</h3>
          <span
            className={`status-chip status-chip-${runtimeInstalled ? "success" : "warning"}`}
          >
            {runtimeInstalled ? "Ready" : "Missing"}
          </span>
        </header>

        {manualRuntimeOverride ? (
          <div
            className="meta-list compact-meta-list"
            title={settingsDraft.whisper.cliPath}
          >
            <div>
              <span className="hint-label">Active runtime</span>
              <strong>Manual override</strong>
              <p className="microcopy">
                Your own path is being used. Clear it to switch back to the versions
                the app manages.
              </p>
              <div className="action-row compact-actions">
                <button
                  type="button"
                  className="ghost"
                  onClick={() => onUpdateSettings({ whisper: { cliPath: "" } })}
                >
                  Automatic runtime selection
                </button>
              </div>
            </div>
          </div>
        ) : installedRuntimeVersions.length > 0 ? (
          // Named "Active runtime" on purpose: updates.rs tells the user to "Select it
          // from Active runtime", and renaming this leaves that message pointing at
          // nothing.
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
                  whisper: { runtimeVersion: nextValue, cliPath: "" },
                })
              }
              title="Choose any installed app-managed Whisper runtime."
            />
          </label>
        ) : null}

        <div
          className={`update-card is-row ${runtimeInstalled ? "current" : "available"}`}
        >
          <div>
            <strong>
              {runtimeInstalled
                ? "Transcription runs on your machine, with no audio leaving it."
                : "Not set up yet. Download it to transcribe recordings."}
            </strong>
            {runtimeInstalled && runtimeUpdateResult ? (
              <p className="microcopy">{runtimeUpdateResult.message}</p>
            ) : null}
          </div>

          <div className="capability-actions">
            {runtimeInstalled ? (
              <>
                {runtimeUpdateVersion ? (
                  <button
                    type="button"
                    onClick={() => void onDownloadRuntimeVersion(runtimeUpdateVersion)}
                    disabled={downloadIsActive || busyAction === "downloadRuntime"}
                  >
                    Update to {runtimeUpdateVersion}
                  </button>
                ) : null}
                <button
                  type="button"
                  className="secondary"
                  onClick={() => void onCheckRuntimeUpdate()}
                  disabled={busyAction === "checkRuntimeUpdate"}
                >
                  {busyAction === "checkRuntimeUpdate" ? "Checking…" : "Check"}
                </button>
              </>
            ) : (
              <button
                type="button"
                onClick={() => void onDownloadRecommendedRuntime()}
                disabled={downloadIsActive || busyAction === "downloadRuntime"}
              >
                Download
              </button>
            )}
          </div>
        </div>

        <DownloadProgressCard
          snapshot={bootstrap.modelDownload}
          kind="runtime"
          downloadIsActive={downloadIsActive}
          onTogglePause={() => void onToggleDownloadPause()}
          onCancel={() => void onCancelDownload()}
        />
      </section>

      <section className="settings-group">
        <header className="settings-group-header">
          <h3>Model</h3>
          <span
            className={`status-chip status-chip-${modelInstalled ? "success" : "warning"}`}
          >
            {modelInstalled ? "Ready" : "Missing"}
          </span>
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
                onUpdateSettings({ whisper: { modelChoice: nextValue } })
              }
              disabled={manualModelOverride}
              title={
                manualModelOverride
                  ? "Clear the manual model override to use app-managed models."
                  : "Choose the app-managed Whisper model."
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
                onUpdateSettings({ whisper: { language: nextValue } })
              }
            />
          </label>

          <label className="field">
            <span>CPU usage during transcription</span>
            <ThemedSelect
              value={settingsDraft.whisper.cpuUsage || "balanced"}
              options={[
                { value: "low", label: "Low" },
                { value: "balanced", label: "Balanced" },
                { value: "high", label: "High" },
              ]}
              placeholder="CPU usage"
              onChange={(nextValue) =>
                onUpdateSettings({ whisper: { cpuUsage: nextValue } })
              }
            />
          </label>

          <label className="field">
            <span>Audio type</span>
            <ThemedSelect
              value={settingsDraft.whisper.audioType || "speech"}
              options={[
                { value: "speech", label: "Speech" },
                { value: "music", label: "Music (songs)" },
              ]}
              placeholder="Audio type"
              onChange={(nextValue) =>
                onUpdateSettings({ whisper: { audioType: nextValue } })
              }
            />
          </label>

          <label className="field">
            <span className="field-label-with-help">
              <span>Transcription speed</span>
              {/* The speed win is measured, but it was measured on two recordings — how the
                  text shifts depends on the audio, so this is flagged rather than presented
                  as a settled improvement. */}
              <span className="field-experimental-tag">Experimental</span>
              <TooltipBadge
                label="?"
                description="Faster narrows the decoder's search from whisper's 5-wide beam to a single greedy pass. On our test recordings it ran 13–23% quicker, and where the text differed it differed sideways — kana instead of kanji, a comma instead of a full stop, a sentence split a word earlier — rather than less accurately. That was two recordings; greedy decoding is generally more prone to repeating a line on difficult audio, so compare before trusting it on something you mine."
              />
            </span>
            <ThemedSelect
              value={settingsDraft.whisper.decodeSpeed || "balanced"}
              options={[
                // "Standard", not "Balanced": CPU usage two rows up already offers a
                // Balanced, and two speed-ish controls sharing a value label with unrelated
                // meanings is a needless trap. The stored value stays "balanced".
                { value: "balanced", label: "Standard (default)" },
                { value: "fast", label: "Faster" },
              ]}
              placeholder="Transcription speed"
              describedBy="transcription-speed-help"
              onChange={(nextValue) =>
                onUpdateSettings({ whisper: { decodeSpeed: nextValue } })
              }
            />
          </label>
        </div>

        {/* Each paragraph names its own control. Without that, the reader takes whichever
            one sits nearest — and this one is about speed, directly under a speed setting
            whose options are not the "higher"/"lower" it describes. */}
        <p className="microcopy">
          CPU usage controls how much of the machine transcription may take: higher uses
          more cores and finishes sooner, lower uses fewer so the machine stays responsive
          while it runs.
        </p>

        <p className="microcopy">
          Set Audio type to Music for songs — it transcribes the whole song including
          sung vocals (Speech mode's voice detection drops singing). Timestamps are a
          little looser in Music mode; keep it on Speech for dialogue.
        </p>

        {/* The id is what the select's aria-describedby points at, so this caveat reaches a
            screen reader — the trigger's aria-label otherwise replaces the whole label and
            the Experimental badge with it. */}
        <p className="microcopy" id="transcription-speed-help">
          Transcription speed is <strong>experimental</strong>: Faster was 13&ndash;23%
          quicker on our test recordings without reading less accurately, but that was two
          recordings and your audio may behave differently. Compare both on a long episode
          before relying on it; Standard is unchanged from before this setting existed.
        </p>

        <div
          className="model-summary"
          title={
            manualModelOverride
              ? settingsDraft.whisper.modelPath
              : selectedModel.description
          }
        >
          <strong>
            {manualModelOverride ? "Manual model override" : selectedModel.label}
          </strong>
          {manualModelOverride ? (
            <span>Your own model file is being used.</span>
          ) : (
            <span>
              {selectedModel.diskSize} - {selectedModel.memoryUsage} RAM
            </span>
          )}
        </div>

        <div
          className={`update-card is-row ${modelInstalled ? "current" : "available"}`}
        >
          <div>
            <strong>
              {modelInstalled
                ? "The model decides how accurate transcription is and how much memory it needs."
                : `Not set up yet. Download the ${selectedModel.label} model to transcribe.`}
            </strong>
            {modelInstalled && modelUpdateResult ? (
              <p className="microcopy">{modelUpdateResult.message}</p>
            ) : null}
          </div>

          <div className="capability-actions">
            {modelInstalled ? (
              <>
                {/* A found update used to be a dead end: the result said a newer file was
                    available and nothing anywhere could fetch it. The model URL is fixed,
                    so downloading the recommended model again IS the update. */}
                {modelUpdateAvailable ? (
                  <button
                    type="button"
                    onClick={() => void onDownloadRecommendedModel()}
                    disabled={downloadIsActive || busyAction === "downloadModel"}
                  >
                    Update
                  </button>
                ) : null}
                {bootstrap.whisperDetection.modelManaged ? (
                  <button
                    type="button"
                    className="secondary"
                    onClick={() => void onCheckModelUpdate()}
                    disabled={busyAction === "checkModelUpdate"}
                  >
                    {busyAction === "checkModelUpdate" ? "Checking…" : "Check"}
                  </button>
                ) : (
                  <span className="microcopy">Your own file — nothing to check.</span>
                )}
              </>
            ) : (
              <button
                type="button"
                onClick={() => void onDownloadRecommendedModel()}
                disabled={downloadIsActive || busyAction === "downloadModel"}
              >
                Download
              </button>
            )}
          </div>
        </div>

        <DownloadProgressCard
          snapshot={bootstrap.modelDownload}
          kind="model"
          downloadIsActive={downloadIsActive}
          onTogglePause={() => void onToggleDownloadPause()}
          onCancel={() => void onCancelDownload()}
        />
      </section>

      {/* One advanced block, not two. These were separate `<details>` in separate cards
          built from identical markup — same field, same input-with-action, same Browse
          button — differing only in which setting they wrote to. */}
      <details className="disclosure">
        <summary>Use your own files instead</summary>
        <div className="settings-grid">
          <label className="field field-wide">
            <span>Runtime path</span>
            <div className="input-with-action">
              <input
                type="text"
                value={settingsDraft.whisper.cliPath}
                onChange={(event) =>
                  onUpdateSettings({ whisper: { cliPath: event.currentTarget.value } })
                }
                placeholder={resolvedCliPath || "Runtime path"}
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

          <label className="field field-wide">
            <span>Model path</span>
            <div className="input-with-action">
              <input
                type="text"
                value={settingsDraft.whisper.modelPath}
                onChange={(event) =>
                  onUpdateSettings({ whisper: { modelPath: event.currentTarget.value } })
                }
                placeholder={resolvedModelPath || "Model path"}
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
        </div>
      </details>
    </SettingsDisclosure>
  );
}
