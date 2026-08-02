use super::client::{
    anki_connect_health_check, anki_connect_request, anki_offline_message, json_string_array,
};

/// The app's OWN note type — a listening card, deliberately distinct from the Anki
/// Lookup add-on's vocabulary card. The add-on's card is word→meaning (the word on the
/// front); this one is media→text for listening practice: the clip and the audio play on
/// the front, and the sentence, translation, and source are revealed on the back. The two
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
const FIELD_NAMES: [&str; 10] = [
    "Sentence",
    "Audio",
    "Image",
    "Video",
    "Translation",
    "Definition",
    "Reading",
    "SourceURL",
    "Title",
    "Time",
];

/// Card front: the clip or the still, the audio, and nothing that gives the answer away.
///
/// The media belongs here rather than on the back. What is being tested is the SENTENCE —
/// whether you can follow the line — and the picture is the context you would have had
/// watching it, not the answer. Subtitles are rendered by the player and never burned into
/// the clip, so showing it gives nothing away.
///
/// A clip replaces the still when both exist: they are the same moment, and one of them is
/// moving. Every block is `{{#Field}}`-guarded so a note missing any of them still
/// generates a card rather than erroring.
const FRONT_TEMPLATE: &str = r#"<div class="wu-card wu-front">
  {{#Video}}<div class="wu-video">{{Video}}</div>{{/Video}}
  {{^Video}}{{#Image}}<div class="wu-image">{{Image}}</div>{{/Image}}{{/Video}}
  {{#Audio}}<div class="wu-audio">{{Audio}}</div>{{/Audio}}
  <div class="wu-hint">Listen and recall</div>
</div>"#;

/// Card back: the replay, the sentence, the translation, an optional reading, and a small
/// source/title/time row.
///
/// The sentence goes through Anki's `{{furigana:}}` filter because the field stores
/// bracket notation like `漢字[かんじ]`, the way Lapis and Yomitan do — so no markup ever
/// arrives from outside, and the reading is hidden until hover by the CSS below.
///
/// Every block is `{{#Field}}…{{/Field}}`-guarded so a blank field leaves no empty row. It
/// does not use `{{FrontSide}}`: the back shows its own `{{Audio}}` replay without
/// repeating the hint.
const BACK_TEMPLATE: &str = r#"<div class="wu-card wu-back">
  {{#Video}}<div class="wu-video">{{Video}}</div>{{/Video}}
  {{^Video}}{{#Image}}<div class="wu-image">{{Image}}</div>{{/Image}}{{/Video}}
  {{#Audio}}<div class="wu-audio">{{Audio}}</div>{{/Audio}}
  {{#Sentence}}<div class="wu-sentence">{{furigana:Sentence}}</div>{{/Sentence}}
  {{#Reading}}<div class="wu-reading">{{Reading}}</div>{{/Reading}}
  {{#Translation}}<div class="wu-translation">{{Translation}}</div>{{/Translation}}
  {{#Definition}}<div class="wu-definition-field">{{Definition}}</div>{{/Definition}}
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
/* The mined still. Bounded so a card stays readable on a phone without the picture
   pushing the sentence off screen. */
.wu-image { margin: 0 0 10px; }
.wu-image img { max-width: 100%; max-height: 45vh; border-radius: 6px; }
/* The mined clip. An inline <video>, so it plays in the card rather than opening in the
   external player the way a [sound:] video does. */
.wu-video { margin: 0 0 10px; }
.wu-video video { max-width: 100%; max-height: 45vh; border-radius: 6px; }
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
/* Readings on hover, the way the Lapis note type does it: the reading is hidden until
   you ask for it, so the card still tests you. Scoped to .wu-sentence so nothing else on
   the card is affected. The field stores Anki bracket notation and {{furigana:}} builds
   the ruby, so no markup ever comes from outside. */
.wu-sentence rt { visibility: hidden; }
.wu-sentence ruby:hover rt,
.wu-sentence ruby:focus-within rt { visibility: visible; }
/* Touch devices have no hover: a tap reveals instead. */
@media (hover: none) {
  .wu-sentence ruby:active rt { visibility: visible; }
}
.wu-reading { color: var(--wu-muted); font-size: 0.95em; margin-top: 2px; }
.wu-translation {
  color: var(--wu-muted);
  font-size: 1.02em;
  line-height: 1.6;
  margin: 14px auto 0;
  max-width: 32em;
}
/* Dictionary definitions for the words the line was mined for. Set left rather than
   centred like the sentence: it is a list to read down, not a line to take in. */
.wu-definition-field {
  margin: 16px auto 0;
  max-width: 32em;
  text-align: left;
  font-size: 0.95em;
  line-height: 1.55;
}
.wu-definition-field ul {
  margin: 4px 0 0;
  padding-left: 1.2em;
}
.wu-definition-field li {
  margin-bottom: 4px;
}
.wu-definition-field .wu-dict,
.wu-definition-field .wou-dict {
  color: var(--wu-muted);
  font-size: 0.88em;
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
/// The furigana rules, kept apart from `CARD_CSS` so they can be appended to a note type
/// that already exists without touching whatever else is in its styling.
const FURIGANA_CSS: &str = r#"
/* Wonder of U furigana — readings hidden until hover, the way Lapis does it. */
.wu-sentence rt { visibility: hidden; }
.wu-sentence ruby:hover rt,
.wu-sentence ruby:focus-within rt { visibility: visible; }
@media (hover: none) {
  .wu-sentence ruby:active rt { visibility: visible; }
}"#;

/// Marker used to tell whether the furigana rules are already present, so re-running this
/// never appends them twice.
const FURIGANA_CSS_MARKER: &str = ".wu-sentence ruby:hover rt";

/// The picture and clip rules, kept apart from `CARD_CSS` for the same reason the furigana
/// ones are: `CARD_CSS` is only ever written when the note type is CREATED, so a note type
/// that already existed never received a single rule about media — which is how a mined clip
/// ended up rendering at its native size and pushing the card sideways.
const MEDIA_CSS: &str = r#"
/* Wonder of U media — a mined still or clip, bounded so the card stays readable. */
.wu-image { margin: 0 0 10px; }
.wu-image img { max-width: 100%; max-height: 45vh; border-radius: 6px; }
.wu-video { margin: 0 0 10px; }
.wu-video video { max-width: 100%; max-height: 45vh; border-radius: 6px; }"#;

const MEDIA_CSS_MARKER: &str = ".wu-video video";

/// Adds the media blocks a back template is missing, leaving everything else alone.
///
/// Inserted directly after the card's opening tag when there is one, so the picture or clip
/// leads the answer the way the app's own template does; otherwise prepended. A template
/// that already mentions the field is untouched, which is what makes this idempotent and
/// what keeps a hand-arranged layout hand-arranged.
fn ensure_media_blocks(back_html: &str) -> String {
    const BLOCKS: [(&str, &str); 2] = [
        (
            "{{Video}}",
            "  {{#Video}}<div class=\"wu-video\">{{Video}}</div>{{/Video}}",
        ),
        (
            "{{Image}}",
            "  {{#Image}}<div class=\"wu-image\">{{Image}}</div>{{/Image}}",
        ),
    ];

    let mut html = back_html.to_string();
    for (reference, block) in BLOCKS {
        if html.contains(reference) {
            continue;
        }
        match html.find('>') {
            // After the opening tag, so the block lands inside the card's own wrapper
            // rather than outside it.
            Some(position) if html.trim_start().starts_with('<') => {
                html.insert_str(position + 1, &format!("\n{block}"));
            }
            _ => html.insert_str(0, &format!("{block}\n")),
        }
    }
    html
}

/// Brings an EXISTING note type up to date: any field the app writes but the note type
/// lacks, the blocks that render them, the sentence through `{{furigana:}}`, and the hover
/// rules appended to its styling.
///
/// Deliberately surgical rather than a wholesale overwrite. `updateModelStyling` replaces
/// the entire stylesheet, so writing `CARD_CSS` over a note type the user has since
/// customised would silently discard their work. Instead the current template and CSS are
/// read, patched only where needed, and written back — and both patches are idempotent,
/// so clicking the button twice changes nothing the second time.
fn update_existing_note_type() -> Result<(), String> {
    // A field the app writes but the note type does not have is DATA SILENTLY LOST:
    // AnkiConnect drops unknown keys from `addNote` without complaint, so the mine reports
    // success and the picture or clip simply is not there. That is what happened to every
    // note type created before `Image` was added — it kept its original field list forever,
    // because this function only ever patched templates and styling.
    //
    // Fields are appended rather than reordered. `Sentence` must stay first (Anki keys
    // duplicate detection on the first field), and appending cannot disturb that.
    let existing_fields = json_string_array(anki_connect_request(
        "modelFieldNames",
        serde_json::json!({ "modelName": NOTE_TYPE_NAME }),
    )?);
    let mut next_index = existing_fields.len();
    for field_name in FIELD_NAMES {
        if existing_fields.iter().any(|existing| existing == field_name) {
            continue;
        }
        anki_connect_request(
            "modelFieldAdd",
            serde_json::json!({
                "modelName": NOTE_TYPE_NAME,
                "fieldName": field_name,
                "index": next_index,
            }),
        )?;
        next_index += 1;
    }

    let templates = anki_connect_request(
        "modelTemplates",
        serde_json::json!({ "modelName": NOTE_TYPE_NAME }),
    )?;

    if let Some(templates) = templates.as_object() {
        let mut patched = serde_json::Map::new();
        for (name, sides) in templates {
            let Some(sides) = sides.as_object() else {
                continue;
            };
            let mut next = serde_json::Map::new();
            for (side, html) in sides {
                let _ = side;
                let html = html.as_str().unwrap_or_default();
                // Only an unfiltered {{Sentence}} needs rewriting; a template already
                // using the filter (or a hand-edited one) is left exactly as it is.
                let mut updated = html.replace("{{Sentence}}", "{{furigana:Sentence}}");
                // Adding the field is only half of it — a field no template renders shows
                // nothing, and Anki only plays media it can see. BOTH sides get a block for
                // anything they do not already mention: the front is where the clip and the
                // still belong, and the back keeps them so they can be replayed while
                // reading the sentence.
                updated = ensure_media_blocks(&updated);
                next.insert(side.clone(), serde_json::Value::String(updated));
            }
            patched.insert(name.clone(), serde_json::Value::Object(next));
        }
        if !patched.is_empty() {
            anki_connect_request(
                "updateModelTemplates",
                serde_json::json!({
                    "model": { "name": NOTE_TYPE_NAME, "templates": patched }
                }),
            )?;
        }
    }

    let styling = anki_connect_request(
        "modelStyling",
        serde_json::json!({ "modelName": NOTE_TYPE_NAME }),
    )?;
    let current_css = styling
        .get("css")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    // Appended, never overwritten: `updateModelStyling` replaces the whole stylesheet, so
    // writing `CARD_CSS` over a note type the user has customised would discard their work.
    // Accumulated rather than written one block at a time, so adding a third later cannot
    // overwrite the second.
    let mut merged = current_css.to_string();
    if !merged.contains(FURIGANA_CSS_MARKER) {
        merged.push('\n');
        merged.push_str(FURIGANA_CSS);
    }
    if !merged.contains(MEDIA_CSS_MARKER) {
        merged.push('\n');
        merged.push_str(MEDIA_CSS);
    }
    if merged != current_css {
        anki_connect_request(
            "updateModelStyling",
            serde_json::json!({
                "model": { "name": NOTE_TYPE_NAME, "css": merged }
            }),
        )?;
    }

    Ok(())
}

pub(crate) fn create_recommended_note_type_inner() -> Result<String, String> {
    anki_connect_health_check().map_err(|error| anki_offline_message(&error))?;

    let existing = json_string_array(anki_connect_request("modelNames", serde_json::json!({}))?);
    if existing.iter().any(|name| name == NOTE_TYPE_NAME) {
        // The note type predates the furigana change, so bring it up to date rather than
        // leaving the user with `漢字[かんじ]` rendered as literal text.
        update_existing_note_type()?;
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

#[cfg(test)]
mod tests {
    use super::{ensure_media_blocks, BACK_TEMPLATE, FIELD_NAMES, FRONT_TEMPLATE};

    #[test]
    fn the_front_shows_the_media_and_never_the_answer() {
        // The clip and the still are context, not the answer — they belong with the audio,
        // before the reveal. The sentence, its reading and its translation must not.
        assert!(FRONT_TEMPLATE.contains("{{Video}}"));
        assert!(FRONT_TEMPLATE.contains("{{Image}}"));
        assert!(FRONT_TEMPLATE.contains("{{Audio}}"));
        for answer in ["Sentence", "Translation", "Reading", "Definition"] {
            assert!(
                !FRONT_TEMPLATE.contains(&format!("{{{{{answer}}}}}")),
                "{answer} would give the answer away on the front"
            );
        }
    }

    #[test]
    fn the_apps_own_front_template_is_left_alone() {
        // The updater runs over both sides now, so the front has to be a no-op too.
        assert_eq!(ensure_media_blocks(FRONT_TEMPLATE), FRONT_TEMPLATE);
    }

    #[test]
    fn the_apps_own_back_template_is_left_alone() {
        // It already renders both, so re-running the updater must be a no-op — otherwise
        // pressing "Create or update" twice would stack duplicate blocks.
        assert_eq!(ensure_media_blocks(BACK_TEMPLATE), BACK_TEMPLATE);
    }

    #[test]
    fn a_template_predating_the_media_fields_gains_both() {
        // Exactly the shape shipped before Image existed: the field list and the template
        // both stop at audio and sentence.
        let old = "<div class=\"wu-card wu-back\">\n  {{Audio}}\n  {{Sentence}}\n</div>";
        let updated = ensure_media_blocks(old);
        assert!(updated.contains("{{#Video}}"));
        assert!(updated.contains("{{#Image}}"));
        // Everything that was there stays there.
        assert!(updated.contains("{{Audio}}"));
        assert!(updated.contains("{{Sentence}}"));
    }

    #[test]
    fn blocks_land_inside_the_cards_own_wrapper_not_before_it() {
        let old = "<div class=\"wu-card wu-back\">\n  {{Audio}}\n</div>";
        let updated = ensure_media_blocks(old);
        assert!(
            updated.starts_with("<div class=\"wu-card wu-back\">"),
            "the wrapper must still open the template: {updated}"
        );
    }

    #[test]
    fn a_template_that_already_shows_one_only_gains_the_other() {
        let half = "<div>\n  {{#Image}}{{Image}}{{/Image}}\n  {{Audio}}\n</div>";
        let updated = ensure_media_blocks(half);
        assert_eq!(
            updated.matches("{{Image}}").count(),
            half.matches("{{Image}}").count(),
            "an existing Image block must not be duplicated"
        );
        assert!(updated.contains("{{#Video}}"));
    }

    #[test]
    fn a_template_with_no_markup_at_all_still_gets_the_blocks() {
        let updated = ensure_media_blocks("{{Audio}}");
        assert!(updated.contains("{{#Video}}"));
        assert!(updated.contains("{{#Image}}"));
        assert!(updated.contains("{{Audio}}"));
    }

    #[test]
    fn every_field_the_miner_writes_is_declared_on_the_note_type() {
        // The miner inserts by mapped field name, and AnkiConnect drops a key the note type
        // does not have WITHOUT reporting it — which is how screenshots went missing for
        // anyone whose note type predated the Image field.
        for required in [
            "Sentence",
            "Audio",
            "Image",
            "Video",
            "Translation",
            "Definition",
        ] {
            assert!(
                FIELD_NAMES.contains(&required),
                "{required} is written by the miner but missing from the note type"
            );
        }
    }
}
