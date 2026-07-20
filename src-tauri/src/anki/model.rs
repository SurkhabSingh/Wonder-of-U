use super::client::{
    anki_connect_health_check, anki_connect_request, anki_offline_message, json_string_array,
};

/// The app's OWN note type — a listening card, deliberately distinct from the Anki
/// Lookup add-on's vocabulary card. The add-on's card is word→meaning (the word on the
/// front); this one is audio→text for listening practice: only the audio plays on the
/// front, and the sentence, translation, and source are revealed on the back. The two
/// serve opposite workflows, so they are separate note types with separate names and
/// never collide on creation (the earlier "byte-identical shared type" coupling is
/// intentionally severed).
pub(crate) const NOTE_TYPE_NAME: &str = "Wonder of U Listening";
const CARD_TEMPLATE_NAME: &str = "Listening";

/// Field order. `Sentence` is FIRST on purpose: Anki keys duplicate detection on the
/// first field and the app writes the transcript there, so "already mined / already
/// pushed" keeps working. Display is template-driven, so the sentence being field 1
/// does not put it on the front. `Audio` holds the `[sound:...]` the app fills;
/// `Reading` is an optional manual kana aid, left unmapped by default.
const FIELD_NAMES: [&str; 7] = [
    "Sentence",
    "Audio",
    "Translation",
    "Reading",
    "SourceURL",
    "Title",
    "Time",
];

/// Card front: the audio alone, so the reviewer listens and recalls the rest. `{{#Audio}}`
/// guards it so a note that somehow lacks audio still generates a card rather than
/// erroring.
const FRONT_TEMPLATE: &str = r#"<div class="wu-card wu-front">
  {{#Audio}}<div class="wu-audio">{{Audio}}</div>{{/Audio}}
  <div class="wu-hint">Listen and recall</div>
</div>"#;

/// Card back: replay + the sentence (already carrying inline `<ruby>` furigana from the
/// add-on bridge, so it is rendered directly, NOT through Anki's `{{furigana:}}` filter)
/// + translation + an optional reading + a small source/title/time row. Every block is
/// `{{#Field}}…{{/Field}}`-guarded so a blank field leaves no empty row. It does not use
/// `{{FrontSide}}` — the back shows its own `{{Audio}}` replay without repeating the hint.
const BACK_TEMPLATE: &str = r#"<div class="wu-card wu-back">
  {{#Audio}}<div class="wu-audio">{{Audio}}</div>{{/Audio}}
  {{#Sentence}}<div class="wu-sentence">{{Sentence}}</div>{{/Sentence}}
  {{#Reading}}<div class="wu-reading">{{Reading}}</div>{{/Reading}}
  {{#Translation}}<div class="wu-translation">{{Translation}}</div>{{/Translation}}
  <div class="wu-meta">
    {{#Title}}<span class="wu-title">{{Title}}</span>{{/Title}}
    {{#Time}}<span class="wu-time">{{Time}}</span>{{/Time}}
    {{#SourceURL}}<span class="wu-source">{{SourceURL}}</span>{{/SourceURL}}
  </div>
</div>"#;

/// Self-contained styling (Anki cards never see any add-on stylesheet): a clean,
/// Lapis-like light/dark card. The replay button is scaled up via its container; the
/// meta row uses margin, not a border, so a card with no title/source shows no empty
/// rule.
const CARD_CSS: &str = r#".card {
  --wu-bg: #ffffff;
  --wu-fg: #1c1d1f;
  --wu-muted: #6b7280;
  --wu-faint: #9aa1ab;
  --wu-rule: #e6e8eb;
  --wu-accent: #2f6df6;
  font-family: -apple-system, "Segoe UI", "Hiragino Kaku Gothic ProN",
    "Noto Sans JP", "Yu Gothic", Meiryo, system-ui, sans-serif;
  color: var(--wu-fg);
  background: var(--wu-bg);
  padding: 28px 18px;
  line-height: 1.7;
  text-align: center;
}
.nightMode.card {
  --wu-bg: #1e1f22;
  --wu-fg: #eceef1;
  --wu-muted: #a6adba;
  --wu-faint: #7f8794;
  --wu-rule: #33363b;
  --wu-accent: #7aa2ff;
}
.wu-card { max-width: 34em; margin: 0 auto; }
.wu-audio { margin: 8px 0 4px; }
.wu-audio a { transform: scale(1.6); display: inline-block; }
.wu-front .wu-hint {
  margin-top: 22px;
  color: var(--wu-faint);
  font-size: 0.82em;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
.wu-sentence {
  font-size: 1.55em;
  line-height: 2;
  font-weight: 500;
  margin: 18px auto 6px;
  overflow-wrap: anywhere;
}
.wu-back .wu-sentence {
  border-top: 1px solid var(--wu-rule);
  padding-top: 18px;
  margin-top: 14px;
}
.wu-sentence rt {
  font-size: 0.52em;
  color: var(--wu-muted);
  font-weight: 400;
  user-select: none;
}
.wu-reading { color: var(--wu-muted); font-size: 0.95em; margin-top: 2px; }
.wu-translation {
  color: var(--wu-muted);
  font-size: 1.02em;
  line-height: 1.6;
  margin: 14px auto 0;
  max-width: 32em;
}
.wu-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 14px;
  justify-content: center;
  margin-top: 20px;
  color: var(--wu-faint);
  font-size: 0.78em;
}
.wu-meta a { color: var(--wu-accent); text-decoration: none; }
.wu-meta a:hover { text-decoration: underline; }
.wu-title { font-weight: 600; color: var(--wu-muted); }"#;

/// Creates the app's "Wonder of U Listening" note type over AnkiConnect when it is
/// absent. Idempotent: an existing note type of the same name is left untouched (Anki
/// does not enforce unique model names, so the name guard is ours). The distinct name
/// means this never collides with the add-on's "Anki Lookup" vocabulary type. Returns
/// the note type name so the caller can map its role→field settings onto it.
pub(crate) fn create_recommended_note_type_inner() -> Result<String, String> {
    anki_connect_health_check().map_err(|error| anki_offline_message(&error))?;

    let existing = json_string_array(anki_connect_request("modelNames", serde_json::json!({}))?);
    if existing.iter().any(|name| name == NOTE_TYPE_NAME) {
        return Ok(NOTE_TYPE_NAME.to_string());
    }

    anki_connect_request(
        "createModel",
        serde_json::json!({
            "modelName": NOTE_TYPE_NAME,
            "inOrderFields": FIELD_NAMES,
            "css": CARD_CSS,
            "isCloze": false,
            "cardTemplates": [
                {
                    "Name": CARD_TEMPLATE_NAME,
                    "Front": FRONT_TEMPLATE,
                    "Back": BACK_TEMPLATE,
                }
            ],
        }),
    )?;

    Ok(NOTE_TYPE_NAME.to_string())
}
