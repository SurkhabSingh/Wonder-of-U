import { useEffect } from "react";
import type {
  AnkiCatalog,
  AnkiFieldMapping,
  AppBootstrap,
  AppPage,
  AppSettings,
  AutosaveState,
  BusyAction,
  SettingsSection,
  VocabularySuggestions,
  WhisperAssetUpdateResult,
} from "../../types";
import type { RefreshAnkiCatalogOptions } from "../../hooks/useAnkiCatalog";
import { StudyPicksSettingsPage } from "./StudyPicksSettingsPage";
import { ScannerSettingsPage } from "./ScannerSettingsPage";
import { AnkiMappingSettingsPage } from "./AnkiMappingSettingsPage";
import { ModelSettingsPage } from "./ModelSettingsPage";
import { PreferencesSettingsPage } from "./PreferencesSettingsPage";
import { RuntimeSettingsPage } from "./RuntimeSettingsPage";
import type {
  BrowseDirectoryField,
  BrowseFileField,
  SettingsUpdate,
} from "./settingsTypes";
import { StorageSettingsPage } from "./StorageSettingsPage";
import { TranslationSettingsPage } from "./TranslationSettingsPage";
import { WhisperStatusSettingsPage } from "./WhisperStatusSettingsPage";

export function SettingsPages({
  activePage,
  scrollTarget,
  onScrollTargetHandled,
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
  ytdlpUpdateResult,
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
  onDownloadWhisperVadModel,
  onDownloadRecommendedFfmpeg,
  onDownloadRecommendedYtdlp,
  onDownloadRecommendedAlass,
  onDownloadRecommendedDictionary,
  onRefreshKnownWords,
  onScanVocabularySources,
  onCheckYtdlpUpdate,
  onToggleDownloadPause,
  onCancelDownload,
  onRefreshAnkiCatalog,
  onUpdateAnkiField,
}: {
  activePage: AppPage;
  scrollTarget: SettingsSection | null;
  onScrollTargetHandled: () => void;
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
  ytdlpUpdateResult: WhisperAssetUpdateResult | null;
  runtimeInstalled: boolean;
  modelInstalled: boolean;
  resolvedCliPath: string;
  resolvedModelPath: string;
  downloadIsActive: boolean;
  onUpdateSettings: (update: SettingsUpdate) => void;
  onBrowseDirectory: (field: BrowseDirectoryField) => void | Promise<void>;
  onBrowseFile: (field: BrowseFileField) => void | Promise<void>;
  onCheckRuntimeUpdate: () => void | Promise<void>;
  onDownloadRuntimeVersion: (version: string) => void | Promise<void>;
  onDownloadRecommendedRuntime: () => void | Promise<void>;
  onCheckModelUpdate: () => void | Promise<void>;
  onDownloadRecommendedModel: () => void | Promise<void>;
  onDownloadWhisperVadModel: () => void | Promise<void>;
  onDownloadRecommendedFfmpeg: () => void | Promise<void>;
  onDownloadRecommendedYtdlp: () => void | Promise<void>;
  onDownloadRecommendedAlass: () => void | Promise<void>;
  onDownloadRecommendedDictionary: () => void | Promise<void>;
  onRefreshKnownWords: () => void | Promise<void>;
  onScanVocabularySources: () => Promise<VocabularySuggestions | null>;
  onCheckYtdlpUpdate: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  onCancelDownload: () => void | Promise<void>;
  onRefreshAnkiCatalog: (
    noteType?: string,
    options?: RefreshAnkiCatalogOptions,
  ) => void | Promise<void>;
  onUpdateAnkiField: (field: keyof AnkiFieldMapping, value: string) => void;
}) {
  // Deep links from the Setup checklist (and post-download navigation) land on
  // the settings page and ask a specific section to scroll into view.
  useEffect(() => {
    if (activePage !== "settings" || scrollTarget === null) {
      return;
    }
    document
      .getElementById(`settings-${scrollTarget}`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
    onScrollTargetHandled();
  }, [activePage, scrollTarget, onScrollTargetHandled]);

  if (activePage !== "settings") {
    return null;
  }

  return (
    <div className="settings-scroll">
      <article className="panel settings-surface">
        <section id="settings-preferences" className="settings-section">
          <PreferencesSettingsPage
            autosaveMessage={autosaveMessage}
            autosaveState={autosaveState}
            busyAction={busyAction}
            onBrowseDirectory={onBrowseDirectory}
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

        <section
          id="settings-whisper"
          className="settings-section whisper-section"
        >
          <WhisperStatusSettingsPage
            activeRuntimeVersion={activeRuntimeVersion}
            bootstrap={bootstrap}
            manualRuntimeOverride={manualRuntimeOverride}
            settingsDraft={settingsDraft}
          />
          <RuntimeSettingsPage
            activeRuntimeVersion={activeRuntimeVersion}
            bootstrap={bootstrap}
            busyAction={busyAction}
            downloadIsActive={downloadIsActive}
            installedRuntimeVersions={installedRuntimeVersions}
            manualRuntimeOverride={manualRuntimeOverride}
            onBrowseFile={onBrowseFile}
            onCancelDownload={onCancelDownload}
            onCheckRuntimeUpdate={onCheckRuntimeUpdate}
            onDownloadRecommendedRuntime={onDownloadRecommendedRuntime}
            onDownloadRuntimeVersion={onDownloadRuntimeVersion}
            onToggleDownloadPause={onToggleDownloadPause}
            onUpdateSettings={onUpdateSettings}
            resolvedCliPath={resolvedCliPath}
            runtimeInstalled={runtimeInstalled}
            runtimeUpdateResult={runtimeUpdateResult}
            runtimeUpdateVersion={runtimeUpdateVersion}
            settingsDraft={settingsDraft}
          />
          <ModelSettingsPage
            bootstrap={bootstrap}
            busyAction={busyAction}
            downloadIsActive={downloadIsActive}
            modelInstalled={modelInstalled}
            modelUpdateResult={modelUpdateResult}
            onBrowseFile={onBrowseFile}
            onCancelDownload={onCancelDownload}
            onCheckModelUpdate={onCheckModelUpdate}
            onDownloadRecommendedModel={onDownloadRecommendedModel}
            onDownloadWhisperVadModel={onDownloadWhisperVadModel}
            onToggleDownloadPause={onToggleDownloadPause}
            onUpdateSettings={onUpdateSettings}
            resolvedModelPath={resolvedModelPath}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-translation" className="settings-section">
          <TranslationSettingsPage
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-storage" className="settings-section">
          <StorageSettingsPage
            bootstrap={bootstrap}
            busyAction={busyAction}
            downloadIsActive={downloadIsActive}
            ytdlpUpdateResult={ytdlpUpdateResult}
            onCancelDownload={onCancelDownload}
            onDownloadRecommendedFfmpeg={onDownloadRecommendedFfmpeg}
            onDownloadRecommendedYtdlp={onDownloadRecommendedYtdlp}
            onDownloadRecommendedAlass={onDownloadRecommendedAlass}
            onCheckYtdlpUpdate={onCheckYtdlpUpdate}
            onToggleDownloadPause={onToggleDownloadPause}
          />
        </section>

        <section id="settings-anki" className="settings-section">
          <AnkiMappingSettingsPage
            busyAction={busyAction}
            displayedAnkiCatalog={displayedAnkiCatalog}
            onRefreshAnkiCatalog={onRefreshAnkiCatalog}
            onUpdateAnkiField={onUpdateAnkiField}
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-studyPicks" className="settings-section">
          <StudyPicksSettingsPage
            bootstrap={bootstrap}
            busyAction={busyAction}
            displayedAnkiCatalog={displayedAnkiCatalog}
            downloadIsActive={downloadIsActive}
            onCancelDownload={onCancelDownload}
            onDownloadRecommendedDictionary={onDownloadRecommendedDictionary}
            onRefreshKnownWords={onRefreshKnownWords}
            onScanVocabularySources={onScanVocabularySources}
            onToggleDownloadPause={onToggleDownloadPause}
            onUpdateSettings={onUpdateSettings}
            settingsDraft={settingsDraft}
          />
        </section>

        <section id="settings-scanner" className="settings-section">
          <ScannerSettingsPage
            settingsDraft={settingsDraft}
            onUpdateSettings={onUpdateSettings}
          />
        </section>
      </article>
    </div>
  );
}
