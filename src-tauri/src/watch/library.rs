//! The video library: which videos the user has added, and which subtitle each is paired with.
//!
//! Kept apart from the recording library on purpose. A video is watched, subtitled and realigned;
//! a recording is transcribed, translated and mined. They share no actions, so sharing a list
//! would only mean one of them constraining the other.
//!
//! Every mutation goes through `upsert_watched_video` so there is exactly one place that decides
//! what identity means and one place that writes. Identity is the video's own path.

use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    anki::screenshot::capture_screenshot,
    app_runtime::emit_app_snapshot,
    app_state::write_persisted_data,
    app_types::{SharedPersistedState, WatchedVideo},
};

/// Where in the video to grab the thumbnail from, as a fraction of its length.
///
/// Not the first frame: films, episodes and rips almost all open on black or a logo card, so
/// frame zero is the one moment guaranteed to say nothing about the video.
const THUMBNAIL_AT_FRACTION: f64 = 0.10;

/// Cheap insurance for a video whose duration could not be probed (`probe_duration_ms` answers
/// 0 on failure): 10% of nothing is still frame zero, so fall back to a fixed offset instead.
const THUMBNAIL_FALLBACK_MS: u64 = 30_000;

pub(crate) fn thumbnail_at_ms(duration_ms: u64) -> u64 {
    if duration_ms == 0 {
        return THUMBNAIL_FALLBACK_MS;
    }
    (duration_ms as f64 * THUMBNAIL_AT_FRACTION) as u64
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Where a subtitle came from, for the chip in the list.
///
/// Deliberately a plain string rather than an enum crossing the wire: it labels a chip and
/// nothing more, and a value the frontend does not recognise must degrade to "no chip" rather
/// than to a parse error that costs the mapping.
pub(crate) const ORIGIN_PICKED: &str = "picked";
pub(crate) const ORIGIN_JIMAKU: &str = "jimaku";
pub(crate) const ORIGIN_GENERATED: &str = "generated";
pub(crate) const ORIGIN_SYNCED: &str = "synced";

/// Keep a stored origin to the four the chip knows how to draw.
///
/// The frontend supplies this for a picked or downloaded subtitle, so without a gate here the
/// stored value would be whatever it happened to send — and "degrades to no chip" would be a
/// claim rather than a behaviour. Anything unrecognised becomes `None`, which renders as no
/// chip and never as a broken one. The mapping itself is untouched either way: the path is the
/// feature, the origin is decoration.
pub(crate) fn normalize_origin(origin: Option<String>) -> Option<String> {
    let origin = origin?;
    [ORIGIN_PICKED, ORIGIN_JIMAKU, ORIGIN_GENERATED, ORIGIN_SYNCED]
        .into_iter()
        .find(|known| *known == origin)
        .map(str::to_string)
}

/// Below this, there is nothing to come back to. Reopening at 12 seconds is not resuming, it
/// is the beginning with extra steps — and it would put a "resume at 0:12" on the row of a
/// video the user has effectively not watched.
const MINIMUM_RESUME_MS: u64 = 30_000;

/// Past this fraction of the video, treat it as finished.
///
/// Without it, watching an episode to the end leaves a resume point in the credits, so every
/// later open lands there — the papercut this feature exists to remove, reintroduced at the
/// other end. Credits and endings run long, so the cut is generous rather than exact.
const FINISHED_AFTER_FRACTION: f64 = 0.95;

/// The position worth returning to, or `None` when there is not one.
///
/// The whole judgement lives here, and is applied where the position is *written*. Storing an
/// already-judged value is what keeps the player and the library row honest with each other:
/// there is no second copy of this rule for one of them to get wrong, and "the row shows a
/// resume point" and "opening resumes" cannot disagree.
///
/// `duration_ms` comes from mpv rather than from the stored entry, because mpv is playing the
/// file and knows. A duration of 0 means it did not answer; the finished check is skipped
/// rather than guessed, since dividing by an unknown length would decide "finished" at random.
pub(crate) fn resume_point_ms(position_ms: u64, duration_ms: u64) -> Option<u64> {
    if position_ms < MINIMUM_RESUME_MS {
        return None;
    }
    if duration_ms > 0 && position_ms as f64 >= duration_ms as f64 * FINISHED_AFTER_FRACTION {
        return None;
    }
    Some(position_ms)
}

/// Insert or update the entry for `video_path`, then persist and broadcast.
///
/// `mutate` receives the existing entry when there is one and a fresh entry when there is not,
/// so a caller never has to ask which case it is in — the difference between "remember this new
/// video" and "update the one I already have" is not something four call sites should each get
/// right.
pub(crate) fn upsert_watched_video<R: Runtime, F>(
    app: &AppHandle<R>,
    video_path: &str,
    mutate: F,
) -> Result<(), String>
where
    F: FnOnce(&mut WatchedVideo),
{
    let snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the video library.".to_string())?;

        let existing = persisted
            .watched_videos
            .iter_mut()
            .find(|video| video.video_path == video_path);

        match existing {
            Some(video) => mutate(video),
            None => {
                let mut video = WatchedVideo {
                    video_path: video_path.to_string(),
                    title: Path::new(video_path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string),
                    added_at_ms: now_ms(),
                    ..WatchedVideo::default()
                };
                mutate(&mut video);
                // Newest first, matching how the list reads: the video just added is the one
                // being looked for.
                persisted.watched_videos.insert(0, video);
            }
        }

        persisted.clone()
    };

    write_persisted_data(app, &snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}

/// Forget a video, and delete the thumbnail we made for it.
///
/// The user's video is never touched. Only the entry and the still frame this app generated are
/// ours to remove, and the confirm copy in the UI says so.
pub(crate) fn remove_watched_video<R: Runtime>(
    app: &AppHandle<R>,
    video_path: &str,
) -> Result<(), String> {
    let (snapshot, thumbnail) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the video library.".to_string())?;

        let thumbnail = persisted
            .watched_videos
            .iter()
            .find(|video| video.video_path == video_path)
            .and_then(|video| video.thumbnail_path.clone());
        persisted
            .watched_videos
            .retain(|video| video.video_path != video_path);

        (persisted.clone(), thumbnail)
    };

    write_persisted_data(app, &snapshot)?;
    // After the write, so a failed removal cannot leave the entry pointing at a deleted file.
    if let Some(thumbnail) = thumbnail {
        let _ = fs::remove_file(thumbnail);
    }
    emit_app_snapshot(app);
    Ok(())
}

/// Grab a still for the list, into the asset directory rather than the user's video folder.
///
/// Reuses the miner's frame capture rather than repeating its ffmpeg arguments: that one already
/// seeks with `-ss` before `-i` (near-instant), downscales without ever upscaling, and verifies
/// ffmpeg actually wrote bytes — ffmpeg can exit 0 having written nothing.
///
/// Returns `None` on any failure. A missing thumbnail is a film icon in the list; it must never
/// be the reason a video cannot be added.
pub(crate) fn capture_thumbnail(
    ffmpeg_path: &Path,
    video_path: &Path,
    asset_directory: &Path,
    duration_ms: u64,
    added_at_ms: u64,
) -> Option<std::path::PathBuf> {
    let directory = asset_directory.join("thumbnails");
    fs::create_dir_all(&directory).ok()?;

    let stem: String = video_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("video")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .take(60)
        .collect();

    // The timestamp keeps two videos with the same name apart, and re-adding one makes a fresh
    // file rather than silently reusing a still of the old.
    let target = directory.join(format!("{stem}-{added_at_ms}.jpg"));
    capture_screenshot(
        ffmpeg_path,
        video_path,
        thumbnail_at_ms(duration_ms),
        &target,
    )
    .ok()?;
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_origin, resume_point_ms, thumbnail_at_ms, MINIMUM_RESUME_MS, ORIGIN_SYNCED,
        THUMBNAIL_FALLBACK_MS,
    };

    /// A 24-minute episode, the length the feature was described against.
    const EPISODE_MS: u64 = 1_440_000;

    /// The ordinary case: stopped in the middle, come back to the middle.
    #[test]
    fn a_position_in_the_body_of_a_video_is_worth_returning_to() {
        assert_eq!(resume_point_ms(754_000, EPISODE_MS), Some(754_000));
    }

    /// Resuming 12 seconds in is the beginning with extra steps.
    #[test]
    fn the_first_seconds_are_not_a_resume_point() {
        assert_eq!(resume_point_ms(0, EPISODE_MS), None);
        assert_eq!(resume_point_ms(12_000, EPISODE_MS), None);
        assert_eq!(resume_point_ms(MINIMUM_RESUME_MS - 1, EPISODE_MS), None);
        assert_eq!(
            resume_point_ms(MINIMUM_RESUME_MS, EPISODE_MS),
            Some(MINIMUM_RESUME_MS)
        );
    }

    /// Watching to the end must CLEAR the point, not park it in the credits — otherwise every
    /// later open lands at the end, which is the papercut this feature removes, mirrored.
    #[test]
    fn finishing_a_video_leaves_no_resume_point() {
        assert_eq!(resume_point_ms(EPISODE_MS, EPISODE_MS), None);
        assert_eq!(resume_point_ms(1_400_000, EPISODE_MS), None);
        // Just inside the cut still counts as unfinished.
        assert_eq!(resume_point_ms(1_360_000, EPISODE_MS), Some(1_360_000));
    }

    /// mpv answering 0 for the duration must not make "finished" a coin flip.
    #[test]
    fn an_unknown_duration_keeps_the_position_rather_than_guessing() {
        assert_eq!(resume_point_ms(754_000, 0), Some(754_000));
        // The minimum still applies: that rule needs no duration.
        assert_eq!(resume_point_ms(12_000, 0), None);
    }

    /// A position past the end of the file — a video replaced by a shorter cut — reads as
    /// finished, so the next open starts over instead of seeking past the end.
    #[test]
    fn a_position_beyond_the_end_is_treated_as_finished() {
        assert_eq!(resume_point_ms(EPISODE_MS + 60_000, EPISODE_MS), None);
    }

    /// An origin the chip cannot draw must cost the chip, never the mapping.
    #[test]
    fn an_unknown_origin_is_dropped_rather_than_stored() {
        assert_eq!(
            normalize_origin(Some(ORIGIN_SYNCED.to_string())),
            Some(ORIGIN_SYNCED.to_string())
        );
        assert_eq!(normalize_origin(Some("whatever".to_string())), None);
        assert_eq!(normalize_origin(Some(String::new())), None);
        assert_eq!(normalize_origin(None), None);
    }

    /// Frame zero is black on almost every real video, so the grab is a tenth of the way in.
    #[test]
    fn the_thumbnail_is_taken_past_the_opening_frames() {
        // A 24-minute episode.
        assert_eq!(thumbnail_at_ms(1_440_000), 144_000);
        assert_eq!(thumbnail_at_ms(60_000), 6_000);
        assert!(thumbnail_at_ms(1_440_000) > 0);
    }

    /// An unprobeable duration arrives as 0, and a tenth of 0 is the frame we were avoiding.
    #[test]
    fn an_unknown_duration_falls_back_to_a_fixed_offset() {
        assert_eq!(thumbnail_at_ms(0), THUMBNAIL_FALLBACK_MS);
    }

    /// A clip shorter than the fallback must not seek past its own end.
    #[test]
    fn a_very_short_video_still_seeks_inside_itself() {
        assert_eq!(thumbnail_at_ms(5_000), 500);
        assert!(thumbnail_at_ms(5_000) < 5_000);
    }
}
