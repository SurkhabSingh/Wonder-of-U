//! Which assets the app can download, and the names it calls them by.
//!
//! This exists because the identity of an asset used to be a bare string written by hand in
//! every place that needed it: `snapshot.kind = Some("alass".into())` in the downloader, a
//! `match` arm in `control.rs`, a member of a union in `types.ts`, a card in a settings page.
//! Nothing connected those four lists, and they drifted — alass reached three of them and not
//! the fourth, so an alass download reported progress under a `kind` no component rendered,
//! and (because the card hides itself when the kind does not match) it blanked every other
//! progress card at the same time. Adding the asset to Rust did not fail to compile; it just
//! quietly did not arrive.
//!
//! An enum makes the Rust half of that impossible: a seventh asset does not compile until
//! every label and event name covers it. The wire format is unchanged — serde emits the same
//! six strings the frontend already matches on — so this is a compile-time fix, not a
//! protocol change.

use serde::Serialize;

/// One downloadable asset.
///
/// `rename_all = "camelCase"` is what keeps the emitted values (`"model"`, `"ytdlp"`, …)
/// byte-identical to the strings this replaced. Renaming a variant therefore changes the
/// wire format, and the round-trip test at the bottom of this file is what makes that
/// obvious rather than mysterious.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AssetKind {
    Model,
    Runtime,
    Ffmpeg,
    Ytdlp,
    Alass,
    Dictionary,
}

impl AssetKind {
    /// Every asset, in the order the tests below read most naturally.
    ///
    /// `cfg(test)` rather than an `allow(dead_code)`: nothing in the app iterates the whole
    /// set yet, and a blanket allow here would also hide the *next* thing that goes unused.
    /// The first-run sequence will want this, and the compiler will say so when it does.
    #[cfg(test)]
    pub(crate) const ALL: [AssetKind; 6] = [
        AssetKind::Model,
        AssetKind::Runtime,
        AssetKind::Ffmpeg,
        AssetKind::Ytdlp,
        AssetKind::Alass,
        AssetKind::Dictionary,
    ];

    /// The asset's name as it reads **mid-sentence**: "Cancelling the FFmpeg download…".
    ///
    /// One label, used in one position, on purpose. The code this replaced kept two lists —
    /// one saying "Runtime"/"Model", another saying "runtime"/"model" — and bridged them by
    /// lowercasing, which turned "FFmpeg" into "ffmpeg" in the resume message. Names like
    /// `yt-dlp` and `alass` are lowercase wherever they appear, so no rule about position
    /// can be correct for all six; the sentences are written to suit the name instead.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            AssetKind::Model => "Whisper model",
            AssetKind::Runtime => "Whisper runtime",
            AssetKind::Ffmpeg => "FFmpeg",
            AssetKind::Ytdlp => "yt-dlp",
            AssetKind::Alass => "alass",
            AssetKind::Dictionary => "Japanese dictionary",
        }
    }
}

/// What the card says while a download is held.
///
/// **Two different pieces of code write this message**, and they are not alternatives — they
/// take turns. `control.rs` writes it the moment the user presses Pause; the download thread
/// writes it again when it next reaches the top of its loop and notices. Both land, in either
/// order, so if they word it differently the card visibly changes its mind.
///
/// They did differ: the worker said "FFmpeg download paused." and built the name from its own
/// per-transfer label, which for the model reads "the Small Whisper model" — so the same pause
/// could be announced two ways with two different names for the same asset. One function, one
/// sentence, and the two writers cannot disagree.
/// `label` is always an [`AssetKind::label`]; it is taken as a string only so the one caller
/// that reads the kind out of a snapshot — where it is an `Option` — does not have to invent
/// an answer for a case that cannot happen.
pub(crate) fn paused_message(label: &str) -> String {
    format!("Paused the {label} download.")
}

#[cfg(test)]
mod tests {
    use super::AssetKind;

    /// The frontend matches on these exact strings, and its own list of them is written by
    /// hand in `types.ts`. Nothing can force those two to agree, so this test at least makes
    /// the Rust side an authoritative list to diff against — and turns a variant rename into
    /// a failing test rather than a progress card that silently stops appearing.
    #[test]
    fn every_kind_serialises_to_the_id_the_frontend_expects() {
        let ids: Vec<String> = AssetKind::ALL
            .iter()
            .map(|kind| serde_json::to_value(kind).unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            ids,
            ["model", "runtime", "ffmpeg", "ytdlp", "alass", "dictionary"]
        );
    }

    /// Two assets sharing a label would make "Cancelling the … download" ambiguous, which is
    /// the class of bug this module exists to end.
    #[test]
    fn no_two_assets_share_a_label() {
        let mut labels: Vec<&str> = AssetKind::ALL.iter().map(|kind| kind.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "two assets share a label: {labels:?}");
    }
}
