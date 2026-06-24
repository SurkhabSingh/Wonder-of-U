import type {
  AppBootstrap,
  AppPage,
  AppSettings,
  BusyAction,
  WhisperAssetUpdateResult,
} from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import { UpdateResultCard } from "../ui/UpdateResultCard";
import { DownloadProgressCard } from "./DownloadProgressCard";
import type { BrowseFileField, SettingsUpdate } from "./settingsTypes";

export function RuntimeSettingsPage({
  activePage,
  activeRuntimeVersion,
  bootstrap,
  busyAction,
  downloadIsActive,
  installedRuntimeVersions,
  manualRuntimeOverride,
  onBrowseFile,
  onCancelDownload,
  onCheckRuntimeUpdate,
  onDownloadRecommendedRuntime,
  onDownloadRuntimeVersion,
  onToggleDownloadPause,
  onUpdateSettings,
  resolvedCliPath,
  runtimeInstalled,
  runtimeUpdateResult,
  runtimeUpdateVersion,
  settingsDraft,
}: {
  activePage: AppPage;
  activeRuntimeVersion: string;
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  downloadIsActive: boolean;
  installedRuntimeVersions: string[];
  manualRuntimeOverride: boolean;
  onBrowseFile: (field: BrowseFileField) => void | Promise<void>;
  onCancelDownload: () => void | Promise<void>;
  onCheckRuntimeUpdate: () => void | Promise<void>;
  onDownloadRecommendedRuntime: () => void | Promise<void>;
  onDownloadRuntimeVersion: (version: string) => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  onUpdateSettings: (update: SettingsUpdate) => void;
  resolvedCliPath: string;
  runtimeInstalled: boolean;
  runtimeUpdateResult: WhisperAssetUpdateResult | null;
  runtimeUpdateVersion: string | null;
  settingsDraft: AppSettings;
}) {
  return (
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

      {manualRuntimeOverride ? (
        <div
          className="meta-list compact-meta-list"
          title={settingsDraft.whisper.cliPath}
        >
          <div>
            <span className="hint-label">Active runtime</span>
            <strong>Manual override</strong>
            <p className="microcopy">
              The manual whisper-cli path is being used. Clear it to switch back
              to app-managed versions.
            </p>
            <div className="action-row compact-actions">
              <button
                type="button"
                className="ghost"
                onClick={() =>
                  onUpdateSettings({
                    whisper: {
                      cliPath: "",
                    },
                  })
                }
              >
                Automatic runtime selection
              </button>
            </div>
          </div>
        </div>
      ) : installedRuntimeVersions.length > 0 ? (
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
            title="Choose any installed app-managed Whisper runtime."
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
              value={settingsDraft.whisper.cliPath}
              onChange={(event) =>
                onUpdateSettings({
                  whisper: {
                    cliPath: event.currentTarget.value,
                  },
                })
              }
              placeholder={resolvedCliPath || "whisper-cli path"}
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
  );
}
