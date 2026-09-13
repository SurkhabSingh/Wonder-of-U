use super::client::{
    anki_connect_health_check, anki_connect_request, anki_offline_message, json_string_array,
    unreadable,
};

pub(crate) const NOTE_TYPE_NAME: &str = "Wonder of U Listening";
const CARD_TEMPLATE_NAME: &str = "Listening";

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

const FRONT_TEMPLATE: &str = r#"<div class="wu-card wu-front">
  {{#Video}}<div class="wu-video">{{Video}}</div>{{/Video}}
  {{^Video}}{{#Image}}<div class="wu-image">{{Image}}</div>{{/Image}}{{/Video}}
  {{#Audio}}<div class="wu-audio">{{Audio}}</div>{{/Audio}}
  <div class="wu-hint">Listen and recall</div>
</div>"#;


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
.wu-image { margin: 0 0 10px; }
.wu-image img { max-width: 100%; max-height: 45vh; border-radius: 6px; }
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

.wu-sentence rt { visibility: hidden; }
.wu-sentence ruby:hover rt,
.wu-sentence ruby:focus-within rt { visibility: visible; }

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

.wu-word {
  margin: 16px auto 0;
  font-size: 1.25em;
  font-weight: 600;
}

.wu-definition-field {
  margin: 16px auto 0;
  max-width: 32em;
  text-align: left;
  font-size: 0.95em;
  line-height: 1.55;
}

.wu-definition + .wu-definition { margin-top: 10px; }
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

const FURIGANA_CSS: &str = r#"
.wu-sentence rt { visibility: hidden; }
.wu-sentence ruby:hover rt,
.wu-sentence ruby:focus-within rt { visibility: visible; }
@media (hover: none) {
  .wu-sentence ruby:active rt { visibility: visible; }
}"#;

const FURIGANA_CSS_MARKER: &str = ".wu-sentence ruby:hover rt";


const MEDIA_CSS: &str = r#"
.wu-image { margin: 0 0 10px; }
.wu-image img { max-width: 100%; max-height: 45vh; border-radius: 6px; }
.wu-video { margin: 0 0 10px; }
.wu-video video { max-width: 100%; max-height: 45vh; border-radius: 6px; }"#;

const MEDIA_CSS_MARKER: &str = ".wu-video video";

const DEFINITION_CSS: &str = r#"
.wu-word { margin: 16px auto 0; font-size: 1.25em; font-weight: 600; }
.wu-definition-field { margin: 16px auto 0; max-width: 32em; text-align: left; font-size: 0.95em; line-height: 1.55; }
.wu-definition + .wu-definition { margin-top: 10px; }
.wu-definition-field ul { margin: 4px 0 0; padding-left: 1.2em; }
.wu-definition-field li { margin-bottom: 4px; }
.wu-definition-field .wu-dict, .wu-definition-field .wou-dict { color: var(--wu-muted); font-size: 0.88em; }"#;

const DEFINITION_CSS_MARKER: &str = ".wu-definition-field ul";

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

const META_BLOCK_OPEN: &str = "<div class=\"wu-meta\">";

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

fn ensure_template_blocks(template_html: &str, is_answer_side: bool) -> String {
    let mut html = template_html.to_string();

    for (reference, block) in LEADING_BACK_BLOCKS {
        if html.contains(reference) {
            continue;
        }
        match html.find('>') {
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
fn trailing_insert_at(html: &str) -> Option<usize> {
    html.find(META_BLOCK_OPEN)
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
fn block_start(html: &str, reference: &str) -> Option<usize> {
    html.find(&reference.replace("{{", "{{#"))
        .or_else(|| html.find(reference))
}

/// Moves a block this app wrote back in front of the ones it is meant to precede.
fn restore_trailing_order(html: &str) -> String {
    let mut html = html.to_string();
    for (index, (_, block)) in TRAILING_BACK_BLOCKS.iter().enumerate() {
        let Some(at) = html.find(block) else {
            continue;
        };
        if let Some(target) = first_later_block(&html, index) {
            if target < at {
                html.replace_range(at..at + block.len(), "");
                let target = first_later_block(&html, index).unwrap_or(at);
                html.insert_str(target, block);
            }
        }
    }
    html
}

fn update_existing_note_type() -> Result<(), String> {
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
                let is_answer_side = side.eq_ignore_ascii_case("back");
                let html = html.as_str().unwrap_or_default();
                let mut updated = html.replace("{{Sentence}}", "{{furigana:Sentence}}");
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
    let current_css = styling
        .get("css")
        .and_then(|value| value.as_str())
        .ok_or_else(|| unreadable("card styling"))?;
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
    #[test]
    fn a_template_predating_the_definition_field_gains_its_block() {
        let old = "<div class=\"wu-card wu-back\">
  {{Audio}}
  {{Sentence}}
</div>";
        let updated = patch_back(old);
        assert!(updated.contains("{{#Definition}}"), "{updated}");
        assert!(updated.contains("{{Definition}}"), "{updated}");
        assert!(
            updated.find("{{Sentence}}") < updated.find("{{#Definition}}"),
            "{updated}"
        );
        assert!(updated.trim_end().ends_with("</div>"), "{updated}");
    }

    /// The word introduces the meanings; it does not trail them.
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
        assert_eq!(updated.matches("{{#Word}}").count(), 1, "{updated}");
        assert_eq!(patch_back(&updated), updated);
    }

    #[test]
    fn a_hand_arranged_word_block_is_left_where_it_was_put() {
        let theirs = "<div class=\"wu-card wu-back\">
  {{#Image}}<div class=\"wu-image\">{{Image}}</div>{{/Image}}
  {{#Video}}<div class=\"wu-video\">{{Video}}</div>{{/Video}}
  {{Sentence}}
  {{#Definition}}<div class=\"wu-definition-field\">{{Definition}}</div>{{/Definition}}
  <p class=\"mine\">{{#Word}}{{Word}}{{/Word}}</p>
</div>";
        assert_eq!(patch_back(theirs), theirs);
    }

    #[test]
    fn the_front_never_gains_the_definitions_block() {
        let old = "<div class=\"wu-card wu-front\">
  {{Audio}}
</div>";
        let updated = patch_front(old);
        assert!(!updated.contains("{{Definition}}"), "{updated}");
        assert!(updated.contains("{{#Video}}"), "{updated}");
    }

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
