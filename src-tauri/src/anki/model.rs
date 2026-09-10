use super::client::{
    anki_connect_health_check, anki_connect_request, anki_offline_message, json_string_array,
    unreadable,
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
const FIELD_NAMES: [&str; 11] = [
    "Sentence",
    "Audio",
    "Image",
    "Video",
    "Word",
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
  {{#Word}}<div class="wu-word">{{Word}}</div>{{/Word}}
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
/* The word a card was mined FOR, when it was mined for a word rather than a
   sentence. Bigger than the meanings under it and set apart, because it is the
   thing being learned and the rest is explanation. */
.wu-word {
  margin: 16px auto 0;
  font-size: 1.25em;
  font-weight: 600;
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
/* A line can be mined for more than one new word, and without this their lists run
   straight into each other. */
.wu-definition + .wu-definition { margin-top: 10px; }
.wu-definition-field ul {
  margin: 4px 0 0;
  padding-left: 1.2em;
}
.wu-definition-field li {
  margin-bottom: 4px;
}
/* `wou-` is the prefix the first cards were written with, before this settled on
   the `wu-` the rest of the card uses. Kept so those cards still render. */
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

/// Styling for the definitions block, for note types that predate it. Kept in step
/// with the `.wu-definition-field` rules in `CARD_CSS`.
const DEFINITION_CSS: &str = r#"
.wu-word { margin: 16px auto 0; font-size: 1.25em; font-weight: 600; }
.wu-definition-field { margin: 16px auto 0; max-width: 32em; text-align: left; font-size: 0.95em; line-height: 1.55; }
.wu-definition + .wu-definition { margin-top: 10px; }
.wu-definition-field ul { margin: 4px 0 0; padding-left: 1.2em; }
.wu-definition-field li { margin-bottom: 4px; }
.wu-definition-field .wu-dict, .wu-definition-field .wou-dict { color: var(--wu-muted); font-size: 0.88em; }"#;

const DEFINITION_CSS_MARKER: &str = ".wu-definition-field ul";

/// Blocks that lead the answer: the picture or the clip, the way the app's own
/// template arranges them.
const LEADING_BACK_BLOCKS: [(&str, &str); 2] = [
    (
        "{{Video}}",
        "  {{#Video}}<div class=\"wu-video\">{{Video}}</div>{{/Video}}",
    ),
    (
        "{{Image}}",
        "  {{#Image}}<div class=\"wu-image\">{{Image}}</div>{{/Image}}",
    ),
];

/// The foot of the card. Nothing the answer is made of belongs under it.
const META_BLOCK_OPEN: &str = "<div class=\"wu-meta\">";

/// Blocks that follow the answer. The word a card was mined for, and the meanings
/// that explain it, belong after the sentence — led with, they would sit above the
/// line they are about, and the word alone would give the answer away.
///
/// Word before Definition: the definitions are of the word, so the word introduces
/// them rather than trailing them.
const TRAILING_BACK_BLOCKS: [(&str, &str); 2] = [
    (
        "{{Word}}",
        "  {{#Word}}<div class=\"wu-word\">{{Word}}</div>{{/Word}}\n",
    ),
    (
        "{{Definition}}",
        "  {{#Definition}}<div class=\"wu-definition-field\">{{Definition}}</div>{{/Definition}}\n",
    ),
];

/// Adds the blocks a back template is missing, leaving everything else alone.
///
/// **Adding a field is only half of updating a note type.** `modelFieldAdd` gives an
/// existing note type somewhere to put the data and AnkiConnect then accepts the
/// write without complaint — but a field no template renders shows nothing, so the
/// card looks exactly as it did and the feature reads as doing nothing at all. That
/// is precisely what happened when `Definition` was added: existing note types took
/// the data and displayed none of it, while newly created ones were fine, because
/// only `BACK_TEMPLATE` had learned about the field.
///
/// So every field the miner writes needs an entry in one of the two lists above,
/// and `every_field_the_miner_writes_has_a_block` refuses to let a new one be added
/// without one.
///
/// A template that already mentions the field is untouched, which is what makes this
/// idempotent and what keeps a hand-arranged layout hand-arranged.
///
/// `is_answer_side` is not decoration. The media blocks belong on both sides — the
/// clip is context, not the answer — but the definitions ARE the answer, and the
/// first version of this patched every side it was given and put them on the front.
fn ensure_template_blocks(template_html: &str, is_answer_side: bool) -> String {
    let mut html = template_html.to_string();

    for (reference, block) in LEADING_BACK_BLOCKS {
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

    for (reference, block) in TRAILING_BACK_BLOCKS {
        if !is_answer_side || html.contains(reference) {
            continue;
        }
        match trailing_insert_at(&html) {
            Some(position) => html.insert_str(position, block),
            None => {
                html.push('\n');
                html.push_str(block.trim_end());
            }
        }
    }

    if is_answer_side {
        html = restore_trailing_order(&html);
    }

    html
}

/// Where a trailing block goes in a template that does not have it yet.
///
/// Above the meta row: that row is the foot of the card, and a block under it reads as an
/// afterthought rather than as part of the answer. Putting these in the right order
/// relative to EACH OTHER is not decided here — `restore_trailing_order` has to do that
/// anyway for templates that are already wrong, and one job wants one owner.
///
/// `None` only when there is no wrapper to sit inside, and the caller appends.
fn trailing_insert_at(html: &str) -> Option<usize> {
    html.find(META_BLOCK_OPEN)
        // Before the wrapper's own closing tag, for the same reason the leading blocks go
        // after its opening one: outside it the block escapes whatever layout and styling
        // the card's container provides.
        .or_else(|| html.rfind("</"))
}

/// The start of the earliest block listed after `index` that this template already has.
fn first_later_block(html: &str, index: usize) -> Option<usize> {
    TRAILING_BACK_BLOCKS
        .iter()
        .skip(index + 1)
        .filter_map(|(reference, _)| block_start(html, reference))
        .min()
}

/// Where a field's block STARTS: its guard when it has one, and the bare reference when
/// the template renders the field unguarded.
///
/// The guard is what has to be found. The bare reference sits INSIDE the block, so a
/// sibling inserted at it would be spliced into the middle of one.
fn block_start(html: &str, reference: &str) -> Option<usize> {
    html.find(&reference.replace("{{", "{{#"))
        .or_else(|| html.find(reference))
}

/// Moves a block this app wrote back in front of the ones it is meant to precede.
///
/// A template that already went wrong cannot be put right by inserting more carefully,
/// because the block is present and nothing inserts it again. This repairs those, and only
/// those: it moves text byte-identical to what this file inserts, so a template someone
/// has arranged by hand is left exactly as they arranged it. The block being verbatim is
/// the evidence that nobody has touched it.
fn restore_trailing_order(html: &str) -> String {
    let mut html = html.to_string();
    for (index, (_, block)) in TRAILING_BACK_BLOCKS.iter().enumerate() {
        let Some(at) = html.find(block) else {
            continue;
        };
        if let Some(target) = first_later_block(&html, index) {
            if target < at {
                html.replace_range(at..at + block.len(), "");
                // Recomputed rather than reused: removing the block above moved every
                // position after it, and the old one now points somewhere else.
                //
                // Falling back to where it came from, not to the end of the string: the
                // block was taken out of a wrapper, and putting it back at `html.len()`
                // would leave it outside one — the single thing every other insertion here
                // is careful to avoid. Nothing removed above can remove the block being
                // looked for, so this cannot fire today; it is written so that the day it
                // does, the template is unchanged rather than broken.
                let target = first_later_block(&html, index).unwrap_or(at);
                html.insert_str(target, block);
            }
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
    let existing_fields = json_string_array(
        anki_connect_request(
            "modelFieldNames",
            serde_json::json!({ "modelName": NOTE_TYPE_NAME }),
        )?,
        "field list",
    )?;
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
                // AnkiConnect names the sides "Front" and "Back". Only the back may
                // receive the definitions block, and an unrecognised side name is
                // treated as the front — adding the answer to a side we cannot
                // identify is the failure worth avoiding.
                let is_answer_side = side.eq_ignore_ascii_case("back");
                let html = html.as_str().unwrap_or_default();
                // Only an unfiltered {{Sentence}} needs rewriting; a template already
                // using the filter (or a hand-edited one) is left exactly as it is.
                let mut updated = html.replace("{{Sentence}}", "{{furigana:Sentence}}");
                // Adding the field is only half of it — a field no template renders shows
                // nothing, and Anki only plays media it can see. BOTH sides get a block for
                // anything they do not already mention: the front is where the clip and the
                // still belong, and the back keeps them so they can be replayed while
                // reading the sentence.
                updated = ensure_template_blocks(&updated, is_answer_side);
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
    // Read strictly, because what follows a lenient read here is not a missing warning
    // but a deleted stylesheet. `unwrap_or_default` made an unreadable reply look like a
    // note type carrying no CSS, and the merge below would then write this app's blocks
    // over whatever the user had — defeating, in the one case it mattered, the rule
    // stated immediately beneath it.
    let current_css = styling
        .get("css")
        .and_then(|value| value.as_str())
        .ok_or_else(|| unreadable("card styling"))?;
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
    if !merged.contains(DEFINITION_CSS_MARKER) {
        merged.push('\n');
        merged.push_str(DEFINITION_CSS);
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

    let existing = json_string_array(
        anki_connect_request("modelNames", serde_json::json!({}))?,
        "note type list",
    )?;
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
    use super::{
        ensure_template_blocks, BACK_TEMPLATE, FIELD_NAMES, FRONT_TEMPLATE, LEADING_BACK_BLOCKS,
        TRAILING_BACK_BLOCKS,
    };

    /// The updater runs over both sides, so every helper here has to say which.
    fn patch_front(html: &str) -> String {
        ensure_template_blocks(html, false)
    }
    fn patch_back(html: &str) -> String {
        ensure_template_blocks(html, true)
    }

    #[test]
    fn the_front_shows_the_media_and_never_the_answer() {
        // The clip and the still are context, not the answer — they belong with the audio,
        // before the reveal. The sentence, its reading and its translation must not.
        assert!(FRONT_TEMPLATE.contains("{{Video}}"));
        assert!(FRONT_TEMPLATE.contains("{{Image}}"));
        assert!(FRONT_TEMPLATE.contains("{{Audio}}"));
        for answer in ["Sentence", "Translation", "Reading", "Definition", "Word"] {
            assert!(
                !FRONT_TEMPLATE.contains(&format!("{{{{{answer}}}}}")),
                "{answer} would give the answer away on the front"
            );
        }
    }

    #[test]
    fn the_apps_own_front_template_is_left_alone() {
        // The updater runs over both sides now, so the front has to be a no-op too.
        assert_eq!(patch_front(FRONT_TEMPLATE), FRONT_TEMPLATE);
    }

    #[test]
    fn the_apps_own_back_template_is_left_alone() {
        // It already renders both, so re-running the updater must be a no-op — otherwise
        // pressing "Create or update" twice would stack duplicate blocks.
        assert_eq!(patch_back(BACK_TEMPLATE), BACK_TEMPLATE);
    }

    #[test]
    fn a_template_predating_the_media_fields_gains_both() {
        // Exactly the shape shipped before Image existed: the field list and the template
        // both stop at audio and sentence.
        let old = "<div class=\"wu-card wu-back\">\n  {{Audio}}\n  {{Sentence}}\n</div>";
        let updated = patch_back(old);
        assert!(updated.contains("{{#Video}}"));
        assert!(updated.contains("{{#Image}}"));
        // Everything that was there stays there.
        assert!(updated.contains("{{Audio}}"));
        assert!(updated.contains("{{Sentence}}"));
    }

    #[test]
    fn blocks_land_inside_the_cards_own_wrapper_not_before_it() {
        let old = "<div class=\"wu-card wu-back\">\n  {{Audio}}\n</div>";
        let updated = patch_back(old);
        assert!(
            updated.starts_with("<div class=\"wu-card wu-back\">"),
            "the wrapper must still open the template: {updated}"
        );
    }

    #[test]
    fn a_template_that_already_shows_one_only_gains_the_other() {
        let half = "<div>\n  {{#Image}}{{Image}}{{/Image}}\n  {{Audio}}\n</div>";
        let updated = patch_back(half);
        assert_eq!(
            updated.matches("{{Image}}").count(),
            half.matches("{{Image}}").count(),
            "an existing Image block must not be duplicated"
        );
        assert!(updated.contains("{{#Video}}"));
    }

    #[test]
    fn a_template_with_no_markup_at_all_still_gets_the_blocks() {
        let updated = patch_back("{{Audio}}");
        assert!(updated.contains("{{#Video}}"));
        assert!(updated.contains("{{#Image}}"));
        assert!(updated.contains("{{Audio}}"));
    }


    /// The bug this whole split exists for.
    ///
    /// A note type created before `Definition` existed gets the FIELD from
    /// `modelFieldAdd`, AnkiConnect then accepts the write without complaint, and
    /// the card shows nothing — because only `BACK_TEMPLATE` had learned about the
    /// field and an existing template never did. It reads as the feature doing
    /// nothing at all, which is exactly how it was reported.
    #[test]
    fn a_template_predating_the_definition_field_gains_its_block() {
        let old = "<div class=\"wu-card wu-back\">
  {{Audio}}
  {{Sentence}}
</div>";
        let updated = patch_back(old);
        assert!(updated.contains("{{#Definition}}"), "{updated}");
        assert!(updated.contains("{{Definition}}"), "{updated}");
        // After the sentence it explains, not above it.
        assert!(
            updated.find("{{Sentence}}") < updated.find("{{#Definition}}"),
            "{updated}"
        );
        // And still inside the card's own wrapper.
        assert!(updated.trim_end().ends_with("</div>"), "{updated}");
    }

    /// The word introduces the meanings; it does not trail them.
    ///
    /// A note type that gained `Definition` before `Word` existed already had the
    /// definitions in place, and a block appended at the end landed after them — and
    /// after the meta row, so the answer's own heading rendered below the source line at
    /// the foot of the card.
    #[test]
    fn a_template_that_already_had_the_definitions_gains_the_word_above_them() {
        let old = "<div class=\"wu-card wu-back\">
  {{Sentence}}
  {{#Definition}}<div class=\"wu-definition-field\">{{Definition}}</div>{{/Definition}}
  <div class=\"wu-meta\">
    {{#Title}}<span class=\"wu-title\">{{Title}}</span>{{/Title}}
  </div>
</div>";
        let updated = patch_back(old);
        let word = updated.find("{{#Word}}").expect("the word block is missing");
        let definition = updated
            .find("{{#Definition}}")
            .expect("the definitions block went missing");
        let meta = updated.find("wu-meta").expect("the meta row went missing");
        assert!(word < definition, "the word trails the meanings it introduces:
{updated}");
        assert!(definition < meta, "the answer sits under the source line:
{updated}");
    }

    /// The blocks a template is missing arrive above the meta row, not under it.
    #[test]
    fn a_template_gains_its_blocks_above_the_foot_of_the_card() {
        let old = "<div class=\"wu-card wu-back\">
  {{Sentence}}
  <div class=\"wu-meta\">
    {{#Title}}<span class=\"wu-title\">{{Title}}</span>{{/Title}}
  </div>
</div>";
        let updated = patch_back(old);
        let meta = updated.find("wu-meta").expect("the meta row went missing");
        for block in ["{{#Word}}", "{{#Definition}}"] {
            let at = updated.find(block).unwrap_or_else(|| panic!("{block} is missing"));
            assert!(at < meta, "{block} sits under the foot of the card:
{updated}");
        }
        // And in the order they are listed, not the order they were inserted.
        assert!(
            updated.find("{{#Word}}") < updated.find("{{#Definition}}"),
            "{updated}"
        );
    }

    /// A card already carrying the word after the meanings is put right, because the
    /// block is present and no amount of careful inserting would move it.
    #[test]
    fn a_word_left_trailing_by_an_earlier_version_is_moved_back_above_the_meanings() {
        let wrong = "<div class=\"wu-card wu-back\">
  {{Sentence}}
  {{#Definition}}<div class=\"wu-definition-field\">{{Definition}}</div>{{/Definition}}
  {{#Word}}<div class=\"wu-word\">{{Word}}</div>{{/Word}}
</div>";
        let updated = patch_back(wrong);
        assert!(
            updated.find("{{#Word}}") < updated.find("{{#Definition}}"),
            "the word was left trailing:
{updated}"
        );
        // Once each. A move that copied would render the word twice.
        assert_eq!(updated.matches("{{#Word}}").count(), 1, "{updated}");
        // And running again changes nothing.
        assert_eq!(patch_back(&updated), updated);
    }

    /// A template someone has arranged themselves is theirs. Only text byte-identical to
    /// what this file writes is ever moved, and a hand-written block is not that — so a
    /// word deliberately placed last stays last, however much this file would prefer it
    /// somewhere else.
    #[test]
    fn a_hand_arranged_word_block_is_left_where_it_was_put() {
        let theirs = "<div class=\"wu-card wu-back\">
  {{#Image}}<div class=\"wu-image\">{{Image}}</div>{{/Image}}
  {{#Video}}<div class=\"wu-video\">{{Video}}</div>{{/Video}}
  {{Sentence}}
  {{#Definition}}<div class=\"wu-definition-field\">{{Definition}}</div>{{/Definition}}
  <p class=\"mine\">{{#Word}}{{Word}}{{/Word}}</p>
</div>";
        // Nothing is missing, so nothing is added either — and the hand-written word
        // block keeps the place it was given.
        assert_eq!(patch_back(theirs), theirs);
    }

    /// The definitions ARE the answer. The first version of this patcher ran over
    /// every side it was handed and put them on the front.
    #[test]
    fn the_front_never_gains_the_definitions_block() {
        let old = "<div class=\"wu-card wu-front\">
  {{Audio}}
</div>";
        let updated = patch_front(old);
        assert!(!updated.contains("{{Definition}}"), "{updated}");
        // The media blocks are context, not the answer, so those still apply.
        assert!(updated.contains("{{#Video}}"), "{updated}");
    }

    /// Adding a field to `FIELD_NAMES` is only half of it. Without a block, an
    /// existing note type silently swallows whatever the miner writes there.
    #[test]
    fn every_field_the_miner_writes_has_a_block() {
        for field in ["Image", "Video", "Definition", "Word"] {
            let reference = format!("{{{{{field}}}}}");
            assert!(
                LEADING_BACK_BLOCKS
                    .iter()
                    .chain(TRAILING_BACK_BLOCKS.iter())
                    .any(|(marker, _)| *marker == reference),
                "{field} is written by the miner but no template block renders it"
            );
        }
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
            "Word",
        ] {
            assert!(
                FIELD_NAMES.contains(&required),
                "{required} is written by the miner but missing from the note type"
            );
        }
    }
}
