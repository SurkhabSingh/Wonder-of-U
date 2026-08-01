import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  AnkiCatalog,
  AppBootstrap,
  AppSettings,
  BusyAction,
  VocabularySource,
} from "../../types";
import { isDownloadBusy } from "../../types";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import type { SettingsUpdate } from "./settingsTypes";
import { DownloadProgressCard } from "./DownloadProgressCard";

/**
 * How long a word has to have stuck before it counts. 21 days is Anki's own
 * "mature" line and the default MorphMan and AnkiMorphs both settled on; the
 * others are here because how long something has to stick before you would say
 * you know it is a genuinely personal call.
 */
const INTERVAL_CHOICES = [7, 14, 21, 30, 60, 90];

function intervalLabel(days: number): string {
  if (days === 21) {
    return "21 days (recommended)";
  }
  return days === 1 ? "1 day" : `${days} days`;
}

/**
 * Turns the timestamp into something worth reading. The exact minute matters
 * less than whether this list is from today or from before a month of study.
 */
function builtAgo(builtAtMs: number | null): string | null {
  if (builtAtMs === null) {
    return null;
  }
  const minutes = Math.floor((Date.now() - builtAtMs) / 60000);
  if (minutes < 1) {
    return "just now";
  }
  if (minutes < 60) {
    return minutes === 1 ? "1 minute ago" : `${minutes} minutes ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return hours === 1 ? "1 hour ago" : `${hours} hours ago`;
  }
  const days = Math.floor(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

function statusTone(status: string): "success" | "warning" | "error" {
  if (status === "ready") {
    return "success";
  }
  return status === "offline" ? "error" : "warning";
}

export function StudyPicksSettingsPage({
  bootstrap,
  busyAction,
  displayedAnkiCatalog,
  downloadIsActive,
  onDownloadRecommendedDictionary,
  onRefreshKnownWords,
  onUpdateSettings,
  onCancelDownload,
  onToggleDownloadPause,
  settingsDraft,
}: {
  bootstrap: AppBootstrap;
  busyAction: BusyAction;
  displayedAnkiCatalog: AnkiCatalog;
  downloadIsActive: boolean;
  onDownloadRecommendedDictionary: () => void | Promise<void>;
  onRefreshKnownWords: () => void | Promise<void>;
  onUpdateSettings: (update: SettingsUpdate) => void;
  onCancelDownload: () => void | Promise<void>;
  onToggleDownloadPause: () => void | Promise<void>;
  settingsDraft: AppSettings;
}) {
  const sources = settingsDraft.anki.vocabularySources ?? [];
  const dictionaryReady = bootstrap.dictionaryDetection.status === "ready";
  const knownWords = bootstrap.knownWords;
  const builtWhen = builtAgo(knownWords.builtAtMs);

  // Each row picks its own note type, so each row needs THAT note type's fields —
  // the catalog only carries the fields of the one note type mining pushes to.
  // Cached per note type: a row re-rendering must not mean another round trip to
  // Anki, and two rows on the same note type should cost one.
  const [fieldsByNoteType, setFieldsByNoteType] = useState<
    Record<string, string[]>
  >({});

  const loadFieldsFor = useCallback(
    async (noteType: string) => {
      if (!noteType) {
        return;
      }
      try {
        const catalog = await invoke<AnkiCatalog>("load_anki_catalog", {
          noteType,
        });
        setFieldsByNoteType((current) => ({
          ...current,
          [noteType]: catalog.fields,
        }));
      } catch {
        // Anki closed. The row keeps whatever field is already saved and shows it
        // as a plain option below, so an offline moment cannot silently blank a
        // configured source.
      }
    },
    [],
  );

  // Fetches the fields for note types already chosen, so re-opening settings shows
  // real dropdowns rather than only the saved value.
  useEffect(() => {
    for (const source of sources) {
      if (source.noteType && !(source.noteType in fieldsByNoteType)) {
        void loadFieldsFor(source.noteType);
      }
    }
  }, [sources, fieldsByNoteType, loadFieldsFor]);

  const updateSources = (nextSources: VocabularySource[]) => {
    onUpdateSettings({ anki: { vocabularySources: nextSources } });
  };

  const updateSourceAt = (index: number, change: Partial<VocabularySource>) => {
    updateSources(
      sources.map((source, position) =>
        position === index ? { ...source, ...change } : source,
      ),
    );
  };

  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Anki</p>
          <h2>Study Picks</h2>
        </div>
        <span
          className={`status-chip status-chip-${statusTone(knownWords.status)}`}
          title={knownWords.message}
        >
          {knownWords.status === "ready"
            ? `${knownWords.wordCount} words`
            : knownWords.status === "unconfigured"
              ? "Off"
              : "Needs a refresh"}
        </span>
      </header>

      <div className="info-note">
        <p className="microcopy">
          Point this at the decks you study vocabulary in, and the app can tell you
          which lines of a transcript are just one word beyond what you already know
          &mdash; the sentences worth mining. Nothing here changes your cards; the
          words are only read.
        </p>
      </div>

      <div className={`update-card ${dictionaryReady ? "current" : "available"}`}>
        <strong>{bootstrap.dictionaryDetection.message}</strong>
        <p className="microcopy">
          Japanese runs words together with no spaces, so counting the words in a
          sentence means knowing where each one ends. This is a one-time
          50&nbsp;MB download, and it is only needed for this feature &mdash;
          transcription, mining and everything else work without it.
        </p>
      </div>

      <div className="action-row inline-actions">
        <button
          type="button"
          className={dictionaryReady ? "secondary" : undefined}
          onClick={() => void onDownloadRecommendedDictionary()}
          disabled={downloadIsActive || isDownloadBusy(busyAction)}
        >
          {dictionaryReady
            ? "Re-download the dictionary"
            : "Download the dictionary"}
        </button>
      </div>
      <DownloadProgressCard
        snapshot={bootstrap.modelDownload}
        kind="dictionary"
        downloadIsActive={downloadIsActive}
        onTogglePause={() => void onToggleDownloadPause()}
        onCancel={() => void onCancelDownload()}
      />

      <div className="info-note">
        <span className="field-label-with-help">
          <span>Where your vocabulary lives</span>
          <TooltipBadge
            label="?"
            description="The note types you learn words from, and which field on each holds the word itself. This is separate from the note type cards are pushed to — the deck you mine INTO is rarely the one you read your vocabulary FROM."
          />
        </span>
        {sources.length === 0 ? (
          <p className="microcopy">
            No sources yet. Add one to switch this on.
          </p>
        ) : null}

        {sources.map((source, index) => (
          <div className="settings-grid anki-grid" key={`source-${index}`}>
            <label className="field">
              <span>Note type</span>
              <ThemedSelect
                value={source.noteType}
                options={[
                  { value: "", label: "Choose note type" },
                  // A saved note type Anki has not listed (offline, or renamed)
                  // stays selectable rather than silently resetting to blank.
                  ...(source.noteType &&
                  !displayedAnkiCatalog.noteTypes.includes(source.noteType)
                    ? [{ value: source.noteType, label: source.noteType }]
                    : []),
                  ...displayedAnkiCatalog.noteTypes.map((noteType) => ({
                    value: noteType,
                    label: noteType,
                  })),
                ]}
                placeholder="Choose note type"
                onChange={(noteType) => {
                  // The field belongs to the old note type, so it cannot survive
                  // the change — a stale name would read as a source that finds
                  // nothing rather than as one that needs finishing.
                  updateSourceAt(index, { noteType, field: "" });
                  void loadFieldsFor(noteType);
                }}
              />
            </label>

            <label className="field">
              <span>Word field</span>
              <ThemedSelect
                value={source.field}
                options={[
                  { value: "", label: "Choose field" },
                  ...(source.field &&
                  !(fieldsByNoteType[source.noteType] ?? []).includes(source.field)
                    ? [{ value: source.field, label: source.field }]
                    : []),
                  ...(fieldsByNoteType[source.noteType] ?? []).map((field) => ({
                    value: field,
                    label: field,
                  })),
                ]}
                placeholder="Choose field"
                onChange={(field) => updateSourceAt(index, { field })}
              />
            </label>

            <div className="action-row inline-actions">
              <button
                type="button"
                className="secondary"
                onClick={() =>
                  updateSources(
                    sources.filter((_, position) => position !== index),
                  )
                }
              >
                Remove
              </button>
            </div>
          </div>
        ))}

        <div className="action-row inline-actions">
          <button
            type="button"
            className="secondary"
            onClick={() => updateSources([...sources, { noteType: "", field: "" }])}
          >
            Add a vocabulary source
          </button>
        </div>
      </div>

      <div className="info-note">
        <label className="field">
          <span className="field-label-with-help">
            <span>Counts as known after</span>
            <TooltipBadge
              label="?"
              description="A word counts once Anki is showing it at this spacing or wider. A word you added yesterday, or one you keep forgetting, does not count — which is the point: sentences are only judged against what has actually stuck."
            />
          </span>
          <ThemedSelect
            value={String(settingsDraft.anki.knownWordIntervalDays ?? 21)}
            options={[
              // A hand-edited value that is not one of the presets still shows,
              // rather than the dropdown quietly claiming a number that is not
              // the one in force.
              ...(INTERVAL_CHOICES.includes(
                settingsDraft.anki.knownWordIntervalDays ?? 21,
              )
                ? []
                : [
                    {
                      value: String(settingsDraft.anki.knownWordIntervalDays),
                      label: intervalLabel(settingsDraft.anki.knownWordIntervalDays),
                    },
                  ]),
              ...INTERVAL_CHOICES.map((days) => ({
                value: String(days),
                label: intervalLabel(days),
              })),
            ]}
            placeholder="21 days"
            onChange={(nextValue) =>
              onUpdateSettings({
                anki: { knownWordIntervalDays: Number(nextValue) },
              })
            }
          />
        </label>
        <p className="microcopy">
          Change this and the saved list is rebuilt on the next refresh, since it
          decides which words made the cut.
        </p>
      </div>

      <div
        className={`update-card ${
          knownWords.status === "ready"
            ? "current"
            : knownWords.status === "offline"
              ? "error"
              : "available"
        }`}
      >
        <strong>{knownWords.message}</strong>
        {builtWhen ? (
          <p className="microcopy">Last read from Anki {builtWhen}.</p>
        ) : null}
      </div>

      <div className="action-row inline-actions">
        <button
          type="button"
          className={knownWords.status === "ready" ? "secondary" : undefined}
          onClick={() => void onRefreshKnownWords()}
          disabled={busyAction === "refreshKnownWords" || sources.length === 0}
        >
          {busyAction === "refreshKnownWords"
            ? "Reading your collection…"
            : "Refresh from Anki"}
        </button>
      </div>
      <p className="microcopy">
        Read on demand rather than watched, so studying never waits on us. Refresh
        after a study session for the list to catch up.
      </p>
    </>
  );
}
