import { useEffect, useState } from "react";
import type {
  AnkiCatalog,
  AnkiFieldMapping,
  AppSettings,
  BusyAction,
  LookupDictionaries,
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
  // Creates the app's "Wonder of U Listening" note type over AnkiConnect (a listening
  // card: audio on the front, transcript/translation on the back), then auto-maps our
  // roles onto its fields so mining works with zero manual setup. The transcript maps
  // to the Sentence field (field 1, so Anki dedup still works); furigana has no separate
  // field — it is written into that same Sentence field as Anki bracket notation
  // (漢字[かんじ]) and rendered by the template's {{furigana:}} filter.
  //
  // Re-running this on a note type that already exists UPDATES it rather than skipping,
  // which is how an older note type picks up the furigana filter and hover styling.
  // The dictionaries the add-on can answer from. Loaded only while the feature is
  // on: it is an HTTP call to Anki, and a page nobody is configuring should not make
  // one. Null until it has been asked for, which is what tells the empty case from
  // the not-yet-loaded one.
  const [dictionaries, setDictionaries] = useState<LookupDictionaries | null>(null);
  const definitionsOn = settingsDraft.features.addDefinitionsToMinedCards;
  const chosenIds = settingsDraft.anki.definitionDictionaryIds ?? [];

  useEffect(() => {
    if (!definitionsOn) {
      return;
    }
    void invoke<LookupDictionaries>("lookup_dictionaries")
      .then(setDictionaries)
      .catch(() => {
        // Anki closed, or an add-on too old to have the endpoint. The block below
        // says so; it is not an error worth a toast on a settings page.
        setDictionaries(null);
      });
  }, [definitionsOn]);

  const toggleDictionary = (id: number) => {
    onUpdateSettings({
      anki: {
        definitionDictionaryIds: chosenIds.includes(id)
          ? chosenIds.filter((chosen) => chosen !== id)
          : [...chosenIds, id],
      },
    });
  };

  // Ids that were chosen and are no longer installed. Shown rather than dropped:
  // updating a dictionary gives it a new id, so silently discarding these would mean
  // card meanings quietly stopping the day a dictionary is updated.
  const missingIds = dictionaries
    ? chosenIds.filter(
        (id) => !dictionaries.dictionaries.some((entry) => entry.id === id),
      )
    : [];

  const handleCreateNoteType = async () => {
    try {
      const noteType = await invoke<string>("create_anki_note_type");
      onUpdateSettings({
        anki: {
          noteType,
          fields: {
            transcription: "Sentence",
            furigana: "",
            audio: "Audio",
            translation: "Translation",
            sourcePath: "",
            createdAt: "",
            sourceUrl: "SourceURL",
            title: "Title",
            position: "Time",
            image: "Image",
            video: "Video",
            definition: "Definition",
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

      <div className="info-note">
        <p className="microcopy">
          No matching note type? Create the app&rsquo;s &ldquo;Wonder of U
          Listening&rdquo; note type in one click &mdash; a listening card that plays the
          audio on the front and reveals the screenshot, transcript, translation, and
          source on the back &mdash; and the fields below map automatically. If you already
          have it, this brings it up to date instead: readings render as furigana and stay
          hidden until you hover. Your own styling is kept &mdash; the rules are appended,
          never overwritten.
        </p>
        <button
          type="button"
          className="secondary"
          onClick={() => void handleCreateNoteType()}
          disabled={
            busyAction === "loadAnki" || displayedAnkiCatalog.status !== "ready"
          }
        >
          Create or update the Wonder of U Listening note type
        </button>
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
                    image: "",
                    video: "",
                    definition: "",
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
          label="Sentence / transcript field"
          description="Receives the transcript during push; it renders on the BACK of the listening card. When furigana is enabled or added later, this same field is replaced with ruby HTML, like a Yomitan expression field."
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
          field="image"
          label="Screenshot field"
          description="Optional still from the video at the mined line's moment. Only lines mined while watching a video get one — a mic recording or an audio file is mined without a picture, and so is a video you have since moved."
          currentValue={settingsDraft.anki.fields.image}
          fieldOptions={displayedAnkiCatalog.fields}
          onChange={onUpdateAnkiField}
        />
        <AnkiFieldSelect
          field="video"
          label="Video clip field"
          description="Optional short video of the mined line, cut to the same window as the audio. Like the screenshot, only lines mined while watching a video get one. Leave unmapped to skip it — clips are far larger than stills and sync to AnkiWeb with everything else."
          currentValue={settingsDraft.anki.fields.video}
          fieldOptions={displayedAnkiCatalog.fields}
          onChange={onUpdateAnkiField}
        />
        <AnkiFieldSelect
          field="definition"
          label="Definitions field"
          description="Optional dictionary meanings of the words in the line you don't know yet. Only filled when the setting below is on, and only for words the app can see are new to you."
          currentValue={settingsDraft.anki.fields.definition}
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
        <label className="field">
          <span>Mined clip padding</span>
          <ThemedSelect
            value={String(settingsDraft.anki.clipPaddingMs ?? 250)}
            options={[
              // Surface a hand-edited value that isn't one of the presets, so the
              // dropdown reflects the active padding instead of an empty placeholder.
              ...([0, 100, 250, 500, 750].includes(
                settingsDraft.anki.clipPaddingMs ?? 250,
              )
                ? []
                : [
                    {
                      value: String(settingsDraft.anki.clipPaddingMs),
                      label: `${settingsDraft.anki.clipPaddingMs} ms`,
                    },
                  ]),
              { value: "0", label: "None (0 ms)" },
              { value: "100", label: "100 ms" },
              { value: "250", label: "250 ms (default)" },
              { value: "500", label: "500 ms" },
              { value: "750", label: "750 ms" },
            ]}
            placeholder="Clip padding"
            onChange={(nextValue) =>
              onUpdateSettings({
                anki: { clipPaddingMs: Number(nextValue) },
              })
            }
          />
        </label>
        <p className="microcopy">
          Extra audio kept on each side of a mined sentence&rsquo;s clip so it
          doesn&rsquo;t cut the first or last syllable. Larger values add more lead-in
          and tail; smaller values make tighter clips.
        </p>
      </div>

      <div className="info-note">
        <label className="toggle inline-toggle">
          <input
            type="checkbox"
            checked={settingsDraft.features.addDefinitionsToMinedCards}
            onChange={(event) =>
              onUpdateSettings({
                features: {
                  addDefinitionsToMinedCards: event.currentTarget.checked,
                },
              })
            }
          />
          <span>Add dictionary meanings for the words you don&rsquo;t know yet</span>
        </label>
        <p className="microcopy">
          When a mined line contains a word you haven&rsquo;t learned, its meaning is
          looked up and written to the definitions field above. Needs Anki open and
          your vocabulary set up under Study Picks. If a meaning can&rsquo;t be found
          the card is still made, just without it &mdash; and the mine says so.
        </p>
        {/* A toggle with nowhere to write is a toggle that does nothing, and from the
            outside that is indistinguishable from a broken feature. Say it here rather
            than letting every mined card be the thing that reports it. */}
        {definitionsOn && !settingsDraft.anki.fields.definition ? (
          <p className="microcopy field-warning">
            Map the definitions field above for this to have anywhere to write. On the
            app&rsquo;s own note type, &ldquo;Create or update&rdquo; adds it.
          </p>
        ) : null}

        {definitionsOn ? (
          <div className="dictionary-choice">
            <span className="field-label-with-help">
              <span>Meanings come from</span>
              <TooltipBadge
                label="?"
                description="Which of your dictionaries are allowed to answer for a mined card. Choose none to use all of them in the order Anki already consults them — that order suits reading, where a monolingual dictionary first is what you want, and it is not always what belongs on a card."
              />
            </span>

            {dictionaries === null ? (
              <p className="microcopy">
                Open Anki to choose &mdash; your dictionaries live in the add-on.
              </p>
            ) : dictionaries.status !== "ready" ? (
              <p className="microcopy">{dictionaries.message}</p>
            ) : (
              <>
                <p className="microcopy">
                  {chosenIds.length === 0
                    ? "All of them, in the order Anki consults them."
                    : `${chosenIds.length} chosen. Cards use only these.`}
                </p>
                <div className="dictionary-choice-list">
                  {dictionaries.dictionaries.map((entry) => (
                    <label className="dictionary-choice-row" key={entry.id}>
                      <input
                        type="checkbox"
                        checked={chosenIds.includes(entry.id)}
                        onChange={() => toggleDictionary(entry.id)}
                      />
                      <span className="dictionary-choice-title">{entry.title}</span>
                      <span className="dictionary-choice-count">
                        {entry.termCount > 0
                          ? `${entry.termCount.toLocaleString()} entries`
                          : "no terms"}
                      </span>
                    </label>
                  ))}
                </div>
              </>
            )}

            {missingIds.length > 0 ? (
              <p className="microcopy field-warning">
                {missingIds.length === 1 ? "A dictionary" : `${missingIds.length} dictionaries`}{" "}
                you chose {missingIds.length === 1 ? "is" : "are"} no longer installed
                &mdash; updating a dictionary replaces it with a new one. Tick a
                replacement, then{" "}
                <button
                  type="button"
                  className="link-button"
                  onClick={() =>
                    onUpdateSettings({
                      anki: {
                        definitionDictionaryIds: chosenIds.filter(
                          (id) => !missingIds.includes(id),
                        ),
                      },
                    })
                  }
                >
                  forget the missing {missingIds.length === 1 ? "one" : "ones"}
                </button>
                .
              </p>
            ) : null}
          </div>
        ) : null}
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
          <span>Automatically add furigana to Japanese cards</span>
        </label>
        <p className="microcopy">
          Applies to mined sentences and to whole recordings alike. Anki needs to be
          open; if it is not, the card is still made and you are told furigana was
          skipped.
        </p>
      </div>

      <div className="info-note">
        <strong>
          Listening card: Replay audio -&gt; Audio (Front), transcript -&gt; Sentence
          (Back).
        </strong>
        <p className="microcopy">
          Furigana is applied directly to the sentence/transcript field, not a
          separate field. The Anki replay icon only shows if the audio field is
          visible in the current card side template.
        </p>
      </div>
    </>
  );
}
