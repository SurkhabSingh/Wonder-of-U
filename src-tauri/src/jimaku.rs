//! Jimaku — fetching Japanese subtitle files for a title.
//!
//! Goes through Rust rather than the webview for the same reason the dictionary does: the
//! app's CSP `connect-src` forbids the webview reaching any host. The browser extension can
//! call jimaku.cc directly because its `<all_urls>` host permission bypasses CORS; this app
//! has no such escape hatch, and would not want one.
//!
//! Ported from the extension's four calls (`background.js`), including the parts that look
//! like quirks and are not:
//! - the API key is sent as a bare `Authorization` header, not `Bearer <key>`;
//! - the episode filter parses numbers out of free-form filenames and misses often (absolute
//!   vs per-season numbering), so a filtered search that finds nothing retries unfiltered;
//! - archives are hidden, because the app can only parse a subtitle file, not unpack one.
//!
//! Personal-use API with a 25 requests/60s limit, surfaced as-is rather than retried: a
//! silent retry against a rate limit just spends the next minute's budget too.

use std::time::Duration;

use serde::{Deserialize, Serialize};

const JIMAKU_API_BASE: &str = "https://jimaku.cc/api";
/// Generous next to the dictionary's 4s: this is a public API over the internet, not a
/// process on localhost.
const JIMAKU_TIMEOUT: Duration = Duration::from_secs(20);

/// Extensions the app cannot do anything with. Jimaku hosts archives alongside subtitles,
/// and offering one only to fail at parse time wastes the user's click.
const ARCHIVE_EXTENSIONS: [&str; 4] = [".zip", ".rar", ".7z", ".tar"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JimakuEntry {
    pub(crate) id: i64,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) english_name: Option<String>,
    #[serde(default)]
    pub(crate) japanese_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct JimakuFile {
    pub(crate) name: String,
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) size: Option<u64>,
}

/// A subtitle file is anything that is not an archive.
///
/// Deliberately a denylist rather than an allowlist of `.srt`/`.ass`: Jimaku carries `.vtt`,
/// `.ssa` and oddly-named files that the app's parser handles fine, and an allowlist would
/// hide them for no reason.
pub(crate) fn is_usable_subtitle_file(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    !ARCHIVE_EXTENSIONS
        .iter()
        .any(|extension| lowered.ends_with(extension))
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(JIMAKU_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())
}

/// Turns an HTTP status into something worth showing a user.
///
/// The three that matter are distinguishable and actionable; anything else is reported with
/// its code rather than flattened into "request failed", so an unexpected response is
/// diagnosable from a screenshot.
fn status_error(status: reqwest::StatusCode) -> Option<String> {
    match status.as_u16() {
        200..=299 => None,
        401 | 403 => Some("Jimaku rejected the API key. Check it in Settings.".into()),
        429 => Some("Jimaku's rate limit is reached — wait a minute and try again.".into()),
        code => Some(format!("Jimaku request failed ({code}).")),
    }
}

fn get(api_key: &str, path: &str) -> Result<serde_json::Value, String> {
    if api_key.trim().is_empty() {
        return Err("Add your Jimaku API key in Settings (jimaku.cc/account).".into());
    }
    let response = client()?
        // A bare key, not `Bearer` — Jimaku's scheme, matching the extension.
        .get(format!("{JIMAKU_API_BASE}{path}"))
        .header(reqwest::header::AUTHORIZATION, api_key.trim())
        .send()
        .map_err(|_| "Could not reach Jimaku.".to_string())?;

    if let Some(error) = status_error(response.status()) {
        return Err(error);
    }
    let body = response
        .text()
        .map_err(|error| format!("Jimaku's response could not be read. {error}"))?;
    serde_json::from_str(&body)
        .map_err(|error| format!("Jimaku returned unreadable data. {error}"))
}

pub(crate) fn search_entries(api_key: &str, query: &str) -> Result<Vec<JimakuEntry>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Enter a title to search for.".into());
    }
    let encoded = urlencoding(query);
    let value = get(api_key, &format!("/entries/search?query={encoded}"))?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Files for an entry, optionally narrowed to an episode.
///
/// Retries without the filter when a filtered request finds nothing. Jimaku derives the
/// episode from free-form filenames, so a title numbered per-season will not match an
/// absolute episode number — and showing every file beats showing none.
pub(crate) fn entry_files(
    api_key: &str,
    entry_id: i64,
    episode: Option<u32>,
) -> Result<Vec<JimakuFile>, String> {
    let path = match episode {
        Some(episode) => format!("/entries/{entry_id}/files?episode={episode}"),
        None => format!("/entries/{entry_id}/files"),
    };
    let value = get(api_key, &path)?;
    let mut files: Vec<JimakuFile> = serde_json::from_value(value).unwrap_or_default();

    if files.is_empty() && episode.is_some() {
        let value = get(api_key, &format!("/entries/{entry_id}/files"))?;
        files = serde_json::from_value(value).unwrap_or_default();
    }

    files.retain(|file| is_usable_subtitle_file(&file.name));
    Ok(files)
}

/// Downloads a subtitle file's text.
///
/// The URL is checked against Jimaku's own host before it is fetched. It arrives from a
/// search result rather than from the user, but "the server told us to" is not a reason to
/// send an API key to an arbitrary address.
pub(crate) fn download_file(api_key: &str, url: &str) -> Result<String, String> {
    if !url.starts_with("https://jimaku.cc/") {
        return Err("That download link is not a Jimaku address.".into());
    }
    let response = client()?
        .get(url)
        .header(reqwest::header::AUTHORIZATION, api_key.trim())
        .send()
        .map_err(|_| "Could not download the subtitle file.".to_string())?;

    if let Some(error) = status_error(response.status()) {
        return Err(error);
    }
    response
        .text()
        .map_err(|error| format!("The subtitle file could not be read. {error}"))
}

/// Makes a Jimaku filename safe to write on Windows, keeping its extension.
///
/// The name comes from a third party, so it is attacker-influenced in principle and merely
/// awkward in practice — Jimaku filenames routinely carry `[]`, `/` and full-width
/// punctuation. Path separators are the part that matters: without stripping them a name
/// could escape the directory it was meant to land in.
pub(crate) fn sanitize_subtitle_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) || character.is_control()
            {
                '_'
            } else {
                character
            }
        })
        .collect();
    // Windows also drops trailing dots and spaces when resolving, which would silently
    // change the name on disk.
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "subtitles.srt".to_string()
    } else {
        trimmed.chars().take(180).collect()
    }
}

/// Percent-encodes a query for a URL.
///
/// Hand-rolled rather than adding a dependency for one call. Everything outside the
/// unreserved set is encoded, which matters here because the queries are Japanese titles.
fn urlencoding(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_are_hidden_and_every_subtitle_format_is_kept() {
        assert!(is_usable_subtitle_file("Episode 01.srt"));
        assert!(is_usable_subtitle_file("Episode 01.ass"));
        // Not an allowlist: Jimaku carries these too and the app's parser handles them.
        assert!(is_usable_subtitle_file("Episode 01.vtt"));
        assert!(is_usable_subtitle_file("Episode 01.ssa"));
        assert!(is_usable_subtitle_file("no extension at all"));

        assert!(!is_usable_subtitle_file("Season 1.zip"));
        assert!(!is_usable_subtitle_file("Season 1.RAR"));
        assert!(!is_usable_subtitle_file("Season 1.7z"));
    }

    #[test]
    fn japanese_titles_survive_the_query_encoding() {
        // The reason this is encoded at all: a raw multi-byte title in a URL is a malformed
        // request, and a title with a space is a different one.
        assert_eq!(urlencoding("Steins Gate"), "Steins%20Gate");
        assert_eq!(
            urlencoding("鋼の錬金術師"),
            "%E9%8B%BC%E3%81%AE%E9%8C%AC%E9%87%91%E8%A1%93%E5%B8%AB"
        );
        // Unreserved characters must pass through untouched, or every query grows noise.
        assert_eq!(urlencoding("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn a_downloaded_name_cannot_escape_its_directory() {
        // The name comes from Jimaku, so separators are the part that actually matters.
        assert_eq!(
            sanitize_subtitle_file_name("../../evil.srt"),
            ".._.._evil.srt"
        );
        assert_eq!(
            sanitize_subtitle_file_name(r"sub\dir\ep01.srt"),
            "sub_dir_ep01.srt"
        );
        // The ordinary case must survive intact — brackets and Japanese are everywhere on
        // Jimaku and are perfectly legal in a filename.
        assert_eq!(
            sanitize_subtitle_file_name("[Group] 鋼の錬金術師 - 01.ja.srt"),
            "[Group] 鋼の錬金術師 - 01.ja.srt"
        );
        // Windows silently drops these when resolving, so they are removed rather than kept.
        assert_eq!(sanitize_subtitle_file_name("  ep01.srt.  "), "ep01.srt");
        assert_eq!(sanitize_subtitle_file_name("   "), "subtitles.srt");
    }

    #[test]
    fn the_actionable_statuses_are_distinguished() {
        use reqwest::StatusCode;
        assert!(status_error(StatusCode::OK).is_none());
        assert!(status_error(StatusCode::UNAUTHORIZED)
            .unwrap()
            .contains("API key"));
        assert!(status_error(StatusCode::TOO_MANY_REQUESTS)
            .unwrap()
            .contains("rate limit"));
        // Anything unexpected still carries its code, so a screenshot is diagnosable.
        assert!(status_error(StatusCode::INTERNAL_SERVER_ERROR)
            .unwrap()
            .contains("500"));
    }
}
