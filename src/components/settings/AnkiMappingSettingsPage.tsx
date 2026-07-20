import type {
  AnkiCatalog,
  AnkiFieldMapping,
  AppSettings,
  BusyAction,
} from "../../types";
import type { RefreshAnkiCatalogOptions } from "../../hooks/useAnkiCatalog";
import { ThemedSelect } from "../ui/ThemedSelect";
import { TooltipBadge } from "../ui/Tooltip";
import { AnkiFieldSelect } from "./AnkiFieldSelect";
import type { SettingsUpdate } from "./settingsTypes";
import { invoke } from "@tauri-apps/api/core";

export function AnkiMappingSettingsPage({
  busyAction,
  displayedAnkiCatalog,
  onRefreshAnkiCatalog,
  onUpdateAnkiField,
  onUpdateSettings,
  settingsDraft,
}: {
  busyAction: BusyAction;
  displayedAnkiCatalog: AnkiCatalog;
  onRefreshAnkiCatalog: (
    noteType?: string,
    options?: RefreshAnkiCatalogOptions,
  ) => void | Promise<void>;
  onUpdateAnkiField: (field: keyof AnkiFieldMapping, value: string) => void;
  onUpdateSettings: (update: SettingsUpdate) => void;
  settingsDraft: AppSettings;
}) {
  // Creates the shared "Anki Lookup" note type over AnkiConnect (the same schema the
  // add-on and extension use), then auto-maps our roles onto its fields so mining
  // works with zero manual setup.
  const handleCreateNoteType = async () => {
    try {
      const noteType = await invoke<string>("create_anki_note_type");
      onUpdateSettings({
        anki: {
          noteType,
          fields: {
            transcription: "Expression",
            furigana: "Furigana",
            audio: "Audio",
            translation: "Translation",
            sourcePath: "",
            createdAt: "",
            sourceUrl: "SourceURL",
            title: "Title",
            position: "Time",
          },
        },
      });
      await onRefreshAnkiCatalog(noteType, { notifySuccess: true });
    } catch {
      // Anki offline / rejected — refresh so the status chip reflects reality.
      await onRefreshAnkiCatalog(undefined);
    }
  };

  return (
    <>
      <header className="panel-header">
        <div>
          <p className="panel-kicker">Anki</p>
          <h2>Card Mapping</h2>
        </div>
        <div className="panel-actions">
          <span
            className={`status-chip status-chip-${
              displayedAnkiCatalog.status === "ready"
                ? "success"
                : displayedAnkiCatalog.status === "offline"
                  ? "error"
                  : "warning"
            }`}
            title={displayedAnkiCatalog.message}
          >
            {displayedAnkiCatalog.status === "ready" ? "Ready" : "Saved"}
          </span>
          <button
            type="button"
            className="secondary"
            onClick={() =>
              void onRefreshAnkiCatalog(undefined, { notifySuccess: true })
            }
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
                    sourceUrl: "",
                    title: "",
                    position: "",
                  },
                },
              });
              if (noteType) {
                void onRefreshAnkiCatalog(noteType);
              }
            }}
          />
        </label>

        <div className="info-note">
          <p className="hint">
            No matching note type? Create the shared &ldquo;Anki Lookup&rdquo; note
            type in one click &mdash; the same one the Wonder of U add-on and browser
            extension use &mdash; and the fields below map automatically.
          </p>
          <button
            type="button"
            className="secondary"
            onClick={() => void handleCreateNoteType()}
            disabled={
              busyAction === "loadAnki" ||
              displayedAnkiCatalog.status !== "ready"
            }
          >
            Create the Wonder of U note type
          </button>
        </div>

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
        <AnkiFieldSelect
          field="sourceUrl"
          label="Source link field"
          description="Optional clickable link back to the source. YouTube imports deep-link to the sentence's exact moment; other URLs link plainly; a local recording with no URL is skipped."
          currentValue={settingsDraft.anki.fields.sourceUrl}
          fieldOptions={displayedAnkiCatalog.fields}
          onChange={onUpdateAnkiField}
        />
        <AnkiFieldSelect
          field="title"
          label="Recording title field"
          description="Optional display title of the recording (an imported file's original name, or the file stem for mic recordings)."
          currentValue={settingsDraft.anki.fields.title}
          fieldOptions={displayedAnkiCatalog.fields}
          onChange={onUpdateAnkiField}
        />
        <AnkiFieldSelect
          field="position"
          label="Timestamp field"
          description="Optional timestamp of the sentence within the recording (H:MM:SS)."
          currentValue={settingsDraft.anki.fields.position}
          fieldOptions={displayedAnkiCatalog.fields}
          onChange={onUpdateAnkiField}
        />
      </div>

      <div className="info-note">
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

      <div className="info-note">
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
    </>
  );
}
