import { useEffect, useMemo, useState } from "react";
import type {
  AnkiCatalog,
  AppSettings,
  BusyAction,
  KnownWordsSnapshot,
  VocabularySource,
} from "../../types";
import type { RefreshAnkiCatalogOptions } from "../../hooks/useAnkiCatalog";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import type { SettingsUpdate } from "./settingsTypes";

const STATUS_CHIP: Record<KnownWordsSnapshot["status"], string> = {
  ready: "success",
  empty: "warning",
  offline: "error",
  unconfigured: "warning",
};

function builtAtLabel(builtAtMs: number | null): string {
  if (builtAtMs === null) {
    return "Not built yet.";
  }

  const minutes = Math.floor((Date.now() - builtAtMs) / 60000);
  if (minutes < 1) {
    return "Built just now.";
  }
  if (minutes < 60) {
    return `Built ${minutes} minute${minutes === 1 ? "" : "s"} ago.`;
  }

  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `Built ${hours} hour${hours === 1 ? "" : "s"} ago.`;
  }

  const days = Math.floor(hours / 24);
  return `Built ${days} day${days === 1 ? "" : "s"} ago.`;
}

export function KnownWordsSettingsPage({
  busyAction,
  displayedAnkiCatalog,
  knownWords,
  onRefreshAnkiCatalog,
  onRefreshKnownWords,
  onUpdateSettings,
  settingsDraft,
}: {
  busyAction: BusyAction;
  displayedAnkiCatalog: AnkiCatalog;
  knownWords: KnownWordsSnapshot;
  onRefreshAnkiCatalog: (
    noteType?: string,
    options?: RefreshAnkiCatalogOptions,
  ) => void | Promise<void>;
  onRefreshKnownWords: () => void | Promise<void>;
  onUpdateSettings: (update: SettingsUpdate) => void;
  settingsDraft: AppSettings;
}) {
  // The persisted sources are always complete: the backend drops any row with an
  // unfilled half on save. A row being filled in therefore cannot live in the
  // saved settings — the autosave would normalise it away and it would vanish
  // under the user's cursor. So the editable list is local state, and only the
  // complete rows are pushed to settings. `settingsDraft` stays the truth for the
  // complete rows; the incomplete ones are ours alone until they are finished.
  const committedSources = settingsDraft.anki.vocabularySources;
  const committedKey = useMemo(
    () => JSON.stringify(committedSources),
    [committedSources],
  );

  const [rows, setRows] = useState<VocabularySource[]>(committedSources);

  // Fold in changes to the saved complete rows without losing an in-progress row:
  // the committed rows are authoritative, and any half-filled row we are holding
  // is kept and floated to the end. Runs when the saved rows change — including
  // when a row we just completed comes back from the save, at which point it is
  // already among the committed rows and so is not duplicated.
  useEffect(() => {
    setRows((current) => {
      const incomplete = current.filter(
        (source) => !source.noteType || !source.field,
      );
      return [...committedSources, ...incomplete];
    });
    // committedKey is the value dependency; committedSources is read inside.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [committedKey]);

  const hasReadySource = rows.some((source) => source.noteType && source.field);

  // Push only the finished rows to settings; the half-filled ones stay local.
  function commitComplete(nextRows: VocabularySource[]) {
    onUpdateSettings({
      anki: {
        vocabularySources: nextRows.filter(
          (source) => source.noteType && source.field,
        ),
      },
    });
  }

  function updateSource(index: number, patch: Partial<VocabularySource>) {
    const nextRows = rows.map((source, position) =>
      position === index ? { ...source, ...patch } : source,
    );
    setRows(nextRows);
    commitComplete(nextRows);
  }

  function handleNoteTypeChange(index: number, noteType: string) {
    // Each note type has its own fields, so the field cannot survive the switch —
    // it would point at a field the new note type does not have.
    updateSource(index, { noteType, field: "" });
    if (noteType) {
      // Fetch this note type's fields now, before the debounced save lands, so the
      // field dropdown fills in without a beat's delay.
      void onRefreshAnkiCatalog(undefined, { vocabularyNoteType: noteType });
    }
  }

  function addSource() {
    // Local only: an empty row is not a source yet, so nothing to persist. This
    // is the whole point — a blank row added to settings would be normalised away.
    setRows((current) => [...current, { noteType: "", field: "" }]);
  }

  function removeSource(index: number) {
    const nextRows = rows.filter((_, position) => position !== index);
    setRows(nextRows);
    commitComplete(nextRows);
  }

  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Anki</p>
          <h2>Known Words</h2>
        </div>
        <div className="panel-actions">
          <span
            className={`status-chip status-chip-${STATUS_CHIP[knownWords.status]}`}
            title={knownWords.message}
          >
            {knownWords.status === "ready"
              ? `${knownWords.wordCount.toLocaleString()} words`
              : "Not built"}
          </span>
        </div>
      </header>

      {rows.length === 0 ? (
        <div className="info-note">
          <strong>No vocabulary sources yet.</strong>
          <p className="microcopy">
            Add the note type and field that hold words you already know — usually
            not the note type Wonder of U pushes into. Vocabulary often lives
            across several note types (a Kaishi deck and a Lapis deck, say); add a
            row for each and they are read together. Until you add one, sentence
            ranking stays off.
          </p>
          <div className="action-row inline-actions">
            <button type="button" className="secondary" onClick={addSource}>
              Add vocabulary source
            </button>
          </div>
        </div>
      ) : (
        <>
          {rows.map((source, index) => (
            <div key={index} className="settings-grid anki-grid">
              <label className="field">
                <span className="field-label-with-help">
                  <span>Vocabulary note type</span>
                  {index === 0 ? (
                    <TooltipBadge
                      label="?"
                      description="The note type holding words you have already learned. This is usually not the note type Wonder of U pushes into."
                    />
                  ) : null}
                </span>
                <ThemedSelect
                  value={source.noteType}
                  options={[
                    { value: "", label: "Choose note type" },
                    ...displayedAnkiCatalog.noteTypes.map((noteType) => ({
                      value: noteType,
                      label: noteType,
                    })),
                  ]}
                  placeholder="Choose note type"
                  onChange={(noteType) => handleNoteTypeChange(index, noteType)}
                />
              </label>

              <label className="field">
                <span className="field-label-with-help">
                  <span>Expression field</span>
                  {index === 0 ? (
                    <TooltipBadge
                      label="?"
                      description="The field holding the word itself. Furigana, ruby, and sound tags are stripped automatically, so a Japanese Support field works as-is. A sentence field will not match anything."
                    />
                  ) : null}
                </span>
                <ThemedSelect
                  value={source.field}
                  options={[
                    { value: "", label: "Choose field" },
                    ...(
                      displayedAnkiCatalog.vocabularyFieldMap[source.noteType] ??
                      []
                    ).map((fieldName) => ({
                      value: fieldName,
                      label: fieldName,
                    })),
                  ]}
                  placeholder="Choose field"
                  disabled={!source.noteType}
                  onChange={(field) => updateSource(index, { field })}
                />
              </label>

              <div className="action-row inline-actions">
                <button
                  type="button"
                  className="ghost danger-action"
                  onClick={() => removeSource(index)}
                >
                  Remove
                </button>
              </div>
            </div>
          ))}

          <div className="info-note">
            <div className="action-row inline-actions">
              <button type="button" className="secondary" onClick={addSource}>
                Add another source
              </button>
              <button
                type="button"
                className="secondary"
                onClick={() => void onRefreshKnownWords()}
                disabled={!hasReadySource || busyAction === "knownWords"}
              >
                {busyAction === "knownWords" ? "Reading Anki..." : "Refresh"}
              </button>
              <span className="microcopy">
                {knownWords.status === "ready"
                  ? `${knownWords.wordCount.toLocaleString()} words. ${builtAtLabel(knownWords.builtAtMs)}`
                  : knownWords.message}
              </span>
            </div>
            <p className="microcopy">
              Reading your collection is manual on purpose. Anki cannot tell Wonder
              of U when you add a card, so the list is only as current as the last
              time you pressed Refresh — press it again after a study session.
              Remove every row to turn sentence ranking off.
            </p>
          </div>
        </>
      )}
    </>
  );
}
