use super::client::{
    anki_connect_health_check, anki_connect_request, anki_offline_message, json_string_array,
};

/// The shared note type all three Wonder of U tools target. The Anki Lookup add-on
/// defines the SAME schema in its `notes/recommended.py`; the two must stay
/// byte-identical (same name, same ordered fields, same template + CSS) so a card
/// made by the app, the extension, or the add-on is interchangeable.
pub(crate) const NOTE_TYPE_NAME: &str = "Anki Lookup";
const CARD_TEMPLATE_NAME: &str = "Recognition";

/// Field order. The first ten mirror the add-on; `SourceURL`/`Title`/`Time` are the
/// Wonder of U additions — left blank by the add-on and filled by the app/extension
/// over AnkiConnect, exactly as `Audio` already works.
const FIELD_NAMES: [&str; 13] = [
    "Expression",
    "Reading",
    "Furigana",
    "Glossary",
    "Sentence",
    "Translation",
    "Pitch",
    "Frequency",
    "Audio",
    "Source",
    "SourceURL",
    "Title",
    "Time",
];

/// Card front: the expression alone, so the reviewer recalls the rest.
const FRONT_TEMPLATE: &str = "<div class=\"expression\">{{Expression}}</div>";

/// Card back. `{{#Field}}…{{/Field}}` guards collapse an empty field, so a note made
/// without (say) pitch or a source link leaves no labelled blank. `{{furigana:Furigana}}`
/// is Anki's native filter (expects bracket notation `漢字[かんじ]`).
const BACK_TEMPLATE: &str = concat!(
    "{{FrontSide}}\n",
    "<hr id=\"answer\">\n",
    "{{#Reading}}<div class=\"reading\">{{Reading}}</div>{{/Reading}}\n",
    "{{#Furigana}}<div class=\"furigana\">{{furigana:Furigana}}</div>{{/Furigana}}\n",
    "{{#Pitch}}<div class=\"pitch\">{{Pitch}}</div>{{/Pitch}}\n",
    "{{#Glossary}}<div class=\"glossary\">{{Glossary}}</div>{{/Glossary}}\n",
    "{{#Sentence}}<div class=\"sentence\">{{Sentence}}</div>{{/Sentence}}\n",
    "{{#Translation}}<div class=\"translation\">{{Translation}}</div>{{/Translation}}\n",
    "{{#Frequency}}<div class=\"frequency\">Frequency: {{Frequency}}</div>{{/Frequency}}\n",
    "{{#Audio}}{{Audio}}{{/Audio}}\n",
    "{{#Title}}<div class=\"title\">{{Title}}</div>{{/Title}}\n",
    "{{#Time}}<div class=\"time\">{{Time}}</div>{{/Time}}\n",
    "{{#SourceURL}}<div class=\"source-url\">{{SourceURL}}</div>{{/SourceURL}}",
);

/// Self-contained: Anki cards do not see any add-on stylesheet.
const CARD_CSS: &str = concat!(
    ".card {\n",
    "  font-family: sans-serif;\n",
    "  font-size: 20px;\n",
    "  text-align: center;\n",
    "  color: black;\n",
    "  background: white;\n",
    "}\n",
    ".expression { font-size: 40px; }\n",
    ".reading { color: #666; }\n",
    ".glossary { text-align: left; margin: 12px auto; max-width: 40em; }\n",
    ".sentence { margin: 12px auto; max-width: 40em; }\n",
    ".translation { color: #666; margin: 8px auto; max-width: 40em; }\n",
    ".frequency { color: #999; font-size: 15px; }\n",
    ".title { color: #888; font-size: 14px; margin-top: 10px; }\n",
    ".time { color: #999; font-size: 13px; }\n",
    ".source-url { margin-top: 8px; font-size: 13px; }\n",
    ".nightMode.card { color: white; background: #2b2b2b; }\n",
    ".nightMode .reading, .nightMode .translation { color: #aaa; }",
);

/// Creates the shared "Anki Lookup" note type over AnkiConnect when it is absent.
/// Idempotent: an existing note type of the same name is left untouched (Anki does
/// not enforce unique model names, so the name guard is ours). Returns the note type
/// name so the caller can map its role→field settings onto it.
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
