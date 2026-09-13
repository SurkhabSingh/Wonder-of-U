use serde::Serialize;

/// One downloadable asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AssetKind {
    Model,
    Runtime,
    Ffmpeg,
    Ytdlp,
    Alass,
    Dictionary,
    Mpv,
}

impl AssetKind {
    /// Every asset, in the order the tests below read most naturally.
    #[cfg(test)]
    pub(crate) const ALL: [AssetKind; 7] = [
        AssetKind::Model,
        AssetKind::Runtime,
        AssetKind::Ffmpeg,
        AssetKind::Ytdlp,
        AssetKind::Alass,
        AssetKind::Dictionary,
        AssetKind::Mpv,
    ];

    /// The asset's name as it reads **mid-sentence**: "Cancelling the FFmpeg download…".
    pub(crate) const fn label(self) -> &'static str {
        match self {
            AssetKind::Model => "Whisper model",
            AssetKind::Runtime => "Whisper runtime",
            AssetKind::Ffmpeg => "FFmpeg",
            AssetKind::Ytdlp => "yt-dlp",
            AssetKind::Alass => "alass",
            AssetKind::Dictionary => "Japanese dictionary",
            AssetKind::Mpv => "mpv",
        }
    }
}

/// What the card says while a download is held.
pub(crate) fn paused_message(label: &str) -> String {
    format!("Paused the {label} download.")
}

#[cfg(test)]
mod tests {
    use super::AssetKind;

    #[test]
    fn every_kind_serialises_to_the_id_the_frontend_expects() {
        let ids: Vec<String> = AssetKind::ALL
            .iter()
            .map(|kind| serde_json::to_value(kind).unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            ids,
            ["model", "runtime", "ffmpeg", "ytdlp", "alass", "dictionary", "mpv"]
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
