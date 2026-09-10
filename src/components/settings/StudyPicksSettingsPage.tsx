import { useCallback, useEffect, useState } from "react";
import {
  fieldsForNoteType,
  noteTypeMissingFromAnki,
} from "../../lib/ankiCatalog";
import { invoke } from "@tauri-apps/api/core";

import type {
  AnkiCatalog,
  AppBootstrap,
  AppSettings,
  BusyAction,
  VocabularySource,
  VocabularySuggestion,
  VocabularySuggestions,
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
  onScanVocabularySources,
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
  onScanVocabularySources: () => Promise<VocabularySuggestions | null>;
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
        // Only an answer that reached Anki is cached. An offline catalog resolves
        // rather than failing, carrying an empty field list because nobody was
        // asked — and caching that MARKED THE NOTE TYPE AS ASKED. The cache key is
        // the gate below, so nothing asked again for as long as the page stayed
        // mounted, and opening Anki changed nothing until it was left and
        // re-entered. The empty dropdown itself was not the damage: it is empty
        // either way while Anki is down. Losing the retry was.
        const fields = fieldsForNoteType(catalog, noteType);
        // Anki answered, and has no note type by this name. That is a real answer, so
        // it is cached like any other: the gate below is "have we asked", and asking
        // again cannot produce a different one while the note type stays deleted.
        const answered =
          fields ?? (noteTypeMissingFromAnki(catalog, noteType) ? [] : null);
        if (answered === null) {
          return;
        }
        setFieldsByNoteType((current) => ({
          ...current,
          [noteType]: answered,
        }));
      } catch {
        // The catalog REJECTED rather than resolving — Anki answered its health
        // check and then failed the real call, which is what "collection is not
        // available" looks like while a profile is closed or a sync is running.
        // Nothing is cached, so the gate below stays open; but the only retry
        // signal is the catalog's status string, and that does not change across
        // this window. So a rejection inside a "ready" plateau still leaves the
        // dropdown holding only its saved value until the page is re-entered.
        // Known gap, same shape as the bug above, left rather than fixed here:
        // making it retry needs a per-note-type outcome, and an outcome written
        // into the state the effect depends on is a render loop waiting to happen.
        // The row keeps whatever field is already saved and shows it as a plain
        // option below, so nothing is silently blanked either way.
      }
    },
    [],
  );

  // Fetches the fields for note types already chosen, so re-opening settings shows
  // real dropdowns rather than only the saved value.
  //
  // Skipped only when Anki is KNOWN to be down, and re-run when that changes.
  // "idle" — the catalog before its first load — still asks, so a page opened with
  // Anki already running fills its dropdowns without waiting on the shared catalog.
  // The status is a dependency because it is the signal that asking is worth it
  // again: without it, an offline first visit left the dropdowns empty until the
  // page was unmounted and rebuilt.
  //
  // `.status` and not the catalog object: the poll rebuilds that object every ten
  // seconds, so depending on it would re-run this forever. The string is equal
  // across ticks, so a steady Anki costs nothing.
  useEffect(() => {
    if (displayedAnkiCatalog.status === "offline") {
      return;
    }
    for (const source of sources) {
      if (source.noteType && !(source.noteType in fieldsByNoteType)) {
        void loadFieldsFor(source.noteType);
      }
    }
  }, [sources, fieldsByNoteType, loadFieldsFor, displayedAnkiCatalog.status]);

  const [scan, setScan] = useState<VocabularySuggestions | null>(null);

  const updateSources = (nextSources: VocabularySource[]) => {
    onUpdateSettings({ anki: { vocabularySources: nextSources } });
  };

  const handleScan = async () => {
    const result = await onScanVocabularySources();
    if (result) {
      setScan(result);
      // The suggestions name note types not otherwise chosen, so their fields have
      // not been fetched — do it now, or accepting one shows a dropdown with only
      // the saved value in it.
      for (const suggestion of result.suggestions) {
        void loadFieldsFor(suggestion.noteType);
      }
    }
  };

  // Compared against the live draft rather than the flag the scan came back with:
  // a suggestion accepted a moment ago is already a source, and the scan's own
  // answer is from before that.
  const isAlreadyASource = (suggestion: VocabularySuggestion) =>
    sources.some(
      (source) =>
        source.noteType === suggestion.noteType &&
        source.field === suggestion.field,
    );

  const addSuggestion = (suggestion: VocabularySuggestion) => {
    if (isAlreadyASource(suggestion)) {
      return;
    }
    updateSources([
      ...sources,
      { noteType: suggestion.noteType, field: suggestion.field },
    ]);
  };

  // Fields this note type is already read from by another row.
  //
  // The same pair twice is not a bigger index — the sources are folded into one set, so a
  // duplicate adds no word. What it does add is a second full walk of that note type on
  // every refresh, over AnkiConnect, for a result already in hand. Offering a field that
  // is spoken for is the only way one gets created, so it is not offered.
  const fieldsSpokenFor = (noteType: string, exceptIndex: number) =>
    new Set(
      sources
        .filter(
          (source, position) =>
            position !== exceptIndex && source.noteType === noteType && source.field,
        )
        .map((source) => source.field),
    );

  // A row that has not been finished yet. An unfinished source is not broken — it is
  // dropped before any query is built, so it costs nothing but the space it takes — and
  // one is the ordinary state of a row being filled in. Several are not: the button that
  // makes them asks nothing and reports nothing, so pressing it repeatedly used to leave a
  // stack of identical empty rows with no way to tell which was being worked on.
  const lastSource = sources[sources.length - 1];
  const lastSourceUnfinished =
    lastSource !== undefined && (!lastSource.noteType || !lastSource.field);

  const updateSourceAt = (index: number, change: Partial<VocabularySource>) => {
    updateSources(
      sources.map((source, position) =>
        position === index ? { ...source, ...change } : source,
      ),
    );
  };

  // The one source the scan can never propose. It judges a field by how consistently
  // it is filled, and the mined word field is empty on every card mined from a row
  // rather than from the lookup popup — so any mixed collection scores it under the
  // fill threshold and it is dropped before it reaches the suggestions. That test is
  // right for someone else's deck and wrong for this one, where the mapping is not a
  // guess: it is the setting the cards were pushed with, so it is offered outright.
  const minedNoteType = settingsDraft.anki.noteType;
  const minedWordField = settingsDraft.anki.fields.word;
  const minedWordsAreUncounted =
    minedNoteType !== "" &&
    minedWordField !== "" &&
    !sources.some(
      (source) =>
        source.noteType === minedNoteType && source.field === minedWordField,
    );

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
          disabled={isDownloadBusy(busyAction)}
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
        <div className="settings-block-header">
          <span className="field-label-with-help">
            <span>Where your vocabulary lives</span>
            <TooltipBadge
              label="?"
              description="The note types you learn words from, and which field on each holds the word itself. This is separate from the note type cards are pushed to — the deck you mine INTO is rarely the one you read your vocabulary FROM."
            />
          </span>

          <button
            type="button"
            className={sources.length === 0 ? undefined : "secondary"}
            onClick={() => void handleScan()}
            disabled={busyAction === "scanVocabulary"}
          >
            {busyAction === "scanVocabulary"
              ? "Looking through your collection…"
              : "Find my vocabulary decks"}
          </button>
        </div>

        <p className="microcopy">
          {sources.length === 0
            ? "Nothing set up yet. Look through your collection, or add a source by hand below."
            : "Look through your collection again for anything not listed here."}
        </p>

        {scan ? (
          <div
            className={`update-card ${
              scan.status === "ready"
                ? "available"
                : scan.status === "offline"
                  ? "error"
                  : ""
            }`}
          >
            <strong>{scan.message}</strong>
            {/* Each row carries real values off the user's own cards. The scan can
                tell a word field from a sentence field, but not a deck of single
                kanji from a deck of words — and one look at the samples can. */}
            {scan.suggestions.map((suggestion) => (
              <div
                className="suggestion-row"
                // Separated by a character no Anki note type or field name can
                // contain, so two suggestions cannot collide on one key. Written
                // as an escape rather than typed: a raw NUL in the source makes
                // the whole file binary to grep and every other text tool.
                key={`${suggestion.noteType}\u0000${suggestion.field}`}
              >
                <div className="suggestion-detail">
                  <strong>
                    {suggestion.noteType} &rarr; {suggestion.field}
                  </strong>
                  <p className="microcopy">
                    {suggestion.matureNoteCount.toLocaleString()} words you have
                    held on to &mdash; {suggestion.samples.join("  ·  ")}
                  </p>
                </div>
                <button
                  type="button"
                  className="secondary"
                  onClick={() => addSuggestion(suggestion)}
                  disabled={isAlreadyASource(suggestion)}
                >
                  {isAlreadyASource(suggestion) ? "Added" : "Use this"}
                </button>
              </div>
            ))}
          </div>
        ) : null}

        {minedWordsAreUncounted ? (
          <div className="suggestion-row">
            <div className="suggestion-detail">
              <strong>
                {minedNoteType} &rarr; {minedWordField}
              </strong>
              <p className="microcopy">
                The words you mine here are written to this field, and it is not one
                of the sources below &mdash; so however well you learn them, they are
                not counted among the words you know.
              </p>
            </div>
            <button
              type="button"
              className="secondary"
              onClick={() =>
                updateSources([
                  ...sources,
                  { noteType: minedNoteType, field: minedWordField },
                ])
              }
            >
              Use this
            </button>
          </div>
        ) : null}

        {sources.length === 0 ? (
          <p className="microcopy">
            No sources yet. Add one to switch this on.
          </p>
        ) : null}

        {sources.map((source, index) => (
          <div className="vocabulary-source-row" key={`source-${index}`}>
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
                  // A row always offers the field it is already set to. Without that, two
                  // rows that duplicate each other each hide the other's field, neither can
                  // list its own value, and both dropdowns go blank — showing no field for
                  // a source that has one, on exactly the rows that need correcting.
                  ...(fieldsByNoteType[source.noteType] ?? [])
                    .filter(
                      (field) =>
                        field === source.field ||
                        !fieldsSpokenFor(source.noteType, index).has(field),
                    )
                    .map((field) => ({
                      value: field,
                      label: field,
                    })),
                ]}
                placeholder="Choose field"
                onChange={(field) => updateSourceAt(index, { field })}
              />
            </label>

            <button
              type="button"
              className="secondary vocabulary-source-remove"
              onClick={() =>
                updateSources(sources.filter((_, position) => position !== index))
              }
              aria-label={`Remove ${source.noteType || "this"} source`}
            >
              Remove
            </button>
          </div>
        ))}

        <div className="action-row compact-actions">
          <button
            type="button"
            className="secondary"
            onClick={() => updateSources([...sources, { noteType: "", field: "" }])}
            disabled={lastSourceUnfinished}
            title={
              lastSourceUnfinished
                ? "Finish the source above first — it still needs a note type and a field."
                : undefined
            }
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
        <p className="microcopy">
          Read on demand rather than watched, so studying never waits on us. Refresh
          after a study session for the list to catch up.
          {builtWhen ? ` Last read from Anki ${builtWhen}.` : ""}
        </p>
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
    </>
  );
}
