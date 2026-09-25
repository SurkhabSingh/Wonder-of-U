use std::collections::HashMap;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::app_types::{
    KnownWordsBuild, KnownWordsState, SharedPersistedState, VocabularySource,
};
use crate::progress::day::DayKey;
use crate::progress::vocabulary::{self, WordHistory};

use super::client::{
    anki_cards_to_notes, anki_connect_health_check, anki_find_cards, anki_notes_info,
    anki_offline_message, anki_reviews_of_cards, json_array,
};
use super::known_words::{note_expression, note_type_search};

/// How many cards' logs one request asks for.
const REVIEW_LOG_BATCH: usize = 1_000;
/// How many notes one `notesInfo` call asks for, as everywhere else.
const NOTES_INFO_BATCH: usize = 500;

/// How far back a replay may reach. A review older than this is a broken timestamp rather
/// than a memory, and the day list it would ask for is measured in decades.
const MOST_DAYS: usize = 3_660;

/// One answer in a card's log: when it was given, and the interval in days it set. Anki
/// reports a learning step in negative seconds, which is never a mature interval.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Answer {
    pub(crate) at_ms: i64,
    pub(crate) interval_days: i64,
}

/// A stretch of time, from when it began until the moment it stopped holding. The interval a
/// card carries now has not stopped, so its stretch runs to the end of time.
type Span = (i64, i64);

/// The stretches where this card's interval stood at or above the threshold, oldest first.
fn mature_spans(answers: &mut [Answer], mature_after_days: i64) -> Vec<Span> {
    answers.sort_by_key(|answer| answer.at_ms);
    let mut spans: Vec<Span> = Vec::new();
    for (at, answer) in answers.iter().enumerate() {
        if answer.interval_days < mature_after_days {
            continue;
        }
        let until = answers.get(at + 1).map_or(i64::MAX, |next| next.at_ms);
        match spans.last_mut() {
            Some(last) if last.1 == answer.at_ms => last.1 = until,
            _ => spans.push((answer.at_ms, until)),
        }
    }
    spans
}

/// One word can sit on several cards and several notes, so its stretches are their union.
fn merged(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_unstable();
    let mut merged: Vec<Span> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 <= last.1 => last.1 = last.1.max(span.1),
            _ => merged.push(span),
        }
    }
    merged
}

/// Every day from the first to `today`, with the moment each one gives way to the next.
/// Older than `MOST_DAYS` is dropped rather than drawn.
pub(crate) fn days_until(first: &DayKey, today: &DayKey) -> Vec<(DayKey, i64)> {
    let mut days: Vec<(DayKey, i64)> = Vec::new();
    let mut cursor = Some(first.clone());
    while let Some(day) = cursor {
        if day.as_str() > today.as_str() {
            break;
        }
        if let Some(ends) = day.ends_at_ms() {
            days.push((day.clone(), ends));
        }
        cursor = day.next();
    }
    if days.len() > MOST_DAYS {
        days.drain(..days.len() - MOST_DAYS);
    }
    days
}

/// The words a card carries, keyed by the card it was answered on.
pub(crate) struct Answers {
    by_word: HashMap<String, Vec<Span>>,
}

impl Answers {
    pub(crate) fn new() -> Self {
        Self {
            by_word: HashMap::new(),
        }
    }

    /// Folds one card's log into the words it carries, keeping no reviews: a collection of
    /// ten thousand cards is read a batch at a time.
    pub(crate) fn add(&mut self, words: &[String], answers: &mut [Answer], mature_after_days: u32) {
        let spans = mature_spans(answers, i64::from(mature_after_days));
        if spans.is_empty() {
            return;
        }
        for word in words {
            self.by_word
                .entry(word.clone())
                .or_default()
                .extend(spans.iter().copied());
        }
    }

    pub(crate) fn earliest(&self) -> Option<i64> {
        self.by_word
            .values()
            .filter_map(|spans| spans.iter().map(|span| span.0).min())
            .min()
    }

    /// How many words stood known at the end of each day, oldest first.
    pub(crate) fn daily_counts(&self, days: &[(DayKey, i64)]) -> Vec<(DayKey, u32)> {
        let ends: Vec<i64> = days.iter().map(|(_, ends)| *ends).collect();
        let mut deltas = vec![0_i32; days.len() + 1];
        for spans in self.by_word.values() {
            for (from, to) in merged(spans.clone()) {
                let first = ends.partition_point(|ends| *ends < from);
                let past = ends.partition_point(|ends| *ends < to);
                if first < past {
                    deltas[first] += 1;
                    deltas[past] -= 1;
                }
            }
        }
        let mut running = 0_i32;
        days.iter()
            .enumerate()
            .map(|(at, (day, _))| {
                running += deltas[at];
                (day.clone(), u32::try_from(running).unwrap_or(0))
            })
            .collect()
    }
}

/// What a rebuild did, in the words the page says it in.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WordHistorySnapshot {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) days: usize,
}

/// Replays Anki's review log into a day-by-day word count, and keeps it only when today's
/// replayed count matches the list the app holds. A replay that disagrees with the live
/// list is a replay that has misread the log, and a wrong history is worse than none.
pub(crate) fn rebuild<R: Runtime>(app: &AppHandle<R>) -> Result<WordHistorySnapshot, String> {
    let build = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        KnownWordsBuild::from_anki_settings(&persisted.settings.anki)
    };
    if build.sources.is_empty() {
        return Ok(WordHistorySnapshot {
            status: "unconfigured".into(),
            message: "Add a vocabulary note type and field to fill in your word history.".into(),
            days: 0,
        });
    }
    if let Err(error) = anki_connect_health_check() {
        return Ok(WordHistorySnapshot {
            status: "offline".into(),
            message: anki_offline_message(&error),
            days: 0,
        });
    }

    let live = {
        let state = app.state::<KnownWordsState>();
        let guard = state
            .0
            .lock()
            .map_err(|_| "Could not read your known-word list.".to_string())?;
        guard
            .as_ref()
            .filter(|index| index.build.matches(&build))
            .map(|index| {
                (
                    index.built_at_ms,
                    u32::try_from(index.words.len()).unwrap_or(u32::MAX),
                )
            })
    };
    let Some((built_at_ms, live_words)) = live else {
        return Ok(WordHistorySnapshot {
            status: "unbuilt".into(),
            message: "Refresh your word list first, so the history can be checked against it."
                .into(),
            days: 0,
        });
    };
    // Checked against the day the list was built rather than today: the list is a measurement
    // with a date on it, and every review since then is one the replay knows and it does not.
    let built_on = crate::progress::day::day_key_for_ms(built_at_ms)
        .ok_or_else(|| "Your word list carries a date outside the calendar.".to_string())?;

    let today = crate::progress::day::today();
    let replayed = replay(&build, &today)?;
    if let Some(message) = disagreement(&replayed.days, &built_on, live_words) {
        return Ok(WordHistorySnapshot {
            status: "mismatch".into(),
            message,
            days: 0,
        });
    }

    let days = replayed.days.len();
    let first = replayed
        .days
        .first()
        .map(|(day, _)| day.as_str().to_string());
    let history = WordHistory::new(
        crate::app_runtime::now_ms(),
        today,
        build,
        replayed.days,
    );
    vocabulary::keep(&app.state::<crate::app_types::AppPathsState>().word_history_file, &history)?;
    let _ = app.emit(crate::app_config::PROGRESS_EVENT, ());
    crate::app_runtime::log_event(
        app,
        "INFO",
        "progress.word_history",
        serde_json::json!({ "days": days, "cards": replayed.cards, "words": replayed.words_today }),
    );
    Ok(WordHistorySnapshot {
        status: "ready".into(),
        message: match first {
            Some(first) => format!("Your word history now reaches back to {first}."),
            None => "Your collection holds no answers to replay yet.".to_string(),
        },
        days,
    })
}

/// Why a replay may not be kept: it has to agree with the list the app measured, on the day
/// that list was built. Every review since then is one the replay knows and the list does not.
fn disagreement(days: &[(DayKey, u32)], built_on: &DayKey, live_words: u32) -> Option<String> {
    match days.iter().find(|(day, _)| day == built_on) {
        Some((_, count)) if *count == live_words => None,
        Some((_, count)) => Some(format!(
            "The review log replays to {count} words on {built_on}, the day your list was built, and the list holds {live_words}. Nothing was kept."
        )),
        None => Some(format!(
            "The review log does not reach {built_on}, the day your list was built. Nothing was kept."
        )),
    }
}

/// What a replay found, before anything is kept.
pub(crate) struct Replayed {
    pub(crate) days: Vec<(DayKey, u32)>,
    pub(crate) words_today: u32,
    pub(crate) cards: usize,
}

/// Replays every answer Anki remembers for the note types the word list is built from.
/// One failed batch fails the whole replay: a history missing a note type is not a shorter
/// history, it is a collapse the chart would draw as forgetting.
pub(crate) fn replay(build: &KnownWordsBuild, today: &DayKey) -> Result<Replayed, String> {
    let mut answers = Answers::new();
    let mut cards = 0_usize;
    for source in &build.sources {
        cards += read_source(source, build.mature_after_days, &mut answers)?;
    }
    let Some(earliest) = answers.earliest() else {
        return Ok(Replayed {
            days: Vec::new(),
            words_today: 0,
            cards,
        });
    };
    let first = crate::progress::day::day_key_for_ms(u64::try_from(earliest).unwrap_or(0))
        .ok_or_else(|| "An answer in the review log is dated outside the calendar.".to_string())?;
    let days = answers.daily_counts(&days_until(&first, today));
    Ok(Replayed {
        words_today: days.last().map_or(0, |(_, count)| *count),
        days,
        cards,
    })
}

/// Reads one source's cards, the word each card carries, and every answer given to it.
fn read_source(
    source: &VocabularySource,
    mature_after_days: u32,
    answers: &mut Answers,
) -> Result<usize, String> {
    let card_ids = anki_find_cards(&note_type_search(&source.note_type))?;
    if card_ids.is_empty() {
        return Ok(0);
    }
    let note_ids = anki_cards_to_notes(&card_ids)?;
    if note_ids.len() != card_ids.len() {
        return Err("Anki answered with a different number of notes than cards.".to_string());
    }
    let word_of_note = read_words(&note_ids, &source.field)?;
    let note_of_card: HashMap<i64, i64> = card_ids.iter().copied().zip(note_ids).collect();

    for batch in card_ids.chunks(REVIEW_LOG_BATCH) {
        let logs = anki_reviews_of_cards(batch)?;
        let logs = logs
            .as_object()
            .ok_or_else(|| "Anki's review log was not a list per card.".to_string())?;
        for card in batch {
            let Some(word) = note_of_card.get(card).and_then(|note| word_of_note.get(note)) else {
                continue;
            };
            let Some(log) = logs.get(&card.to_string()) else {
                continue;
            };
            let mut given = read_answers(log)?;
            answers.add(std::slice::from_ref(word), &mut given, mature_after_days);
        }
    }
    Ok(card_ids.len())
}

/// The word each note carries in the source's field, skipping notes whose field is gone.
fn read_words(note_ids: &[i64], field: &str) -> Result<HashMap<i64, String>, String> {
    let mut unique: Vec<i64> = note_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut words = HashMap::new();
    for batch in unique.chunks(NOTES_INFO_BATCH) {
        let notes = anki_notes_info(batch)?;
        for note in json_array(&notes, "note list")? {
            let Some(id) = note.get("noteId").and_then(serde_json::Value::as_i64) else {
                continue;
            };
            if let Some(word) = note_expression(note, field) {
                words.insert(id, word);
            }
        }
    }
    Ok(words)
}

/// One card's answers. A learning step is reported in negative seconds, and reads as the
/// nothing it is worth.
fn read_answers(log: &serde_json::Value) -> Result<Vec<Answer>, String> {
    json_array(log, "one card's review log")?
        .iter()
        .map(|review| {
            let at_ms = review
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| "An answer in the review log has no date.".to_string())?;
            let interval_days = review
                .get("ivl")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0)
                .max(0);
            Ok(Answer {
                at_ms,
                interval_days,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::day::day_key_for_ms;

    const DAY_MS: i64 = 86_400_000;

    /// A moment `days` before `MIDDAY`, so a span lands where a test can name it.
    const MIDDAY: i64 = 1_789_041_600_000;

    fn days_ago(days: i64) -> i64 {
        MIDDAY - days * DAY_MS
    }

    fn day_of(ms: i64) -> DayKey {
        day_key_for_ms(u64::try_from(ms).expect("a positive timestamp")).expect("a day")
    }

    fn answer(days_before: i64, interval_days: i64) -> Answer {
        Answer {
            at_ms: days_ago(days_before),
            interval_days,
        }
    }

    fn counted(answers: &mut [Answer], first: i64, last: i64) -> Vec<(String, u32)> {
        let mut words = Answers::new();
        words.add(&["語".to_string()], answers, 21);
        let days = days_until(&day_of(days_ago(first)), &day_of(days_ago(last)));
        words
            .daily_counts(&days)
            .into_iter()
            .map(|(day, count)| (day.as_str().to_string(), count))
            .collect()
    }

    #[test]
    fn a_word_counts_from_the_day_its_card_matured() {
        let counts = counted(&mut [answer(10, 3), answer(8, 25)], 12, 6);
        assert_eq!(
            counts.iter().map(|(_, count)| *count).collect::<Vec<_>>(),
            [0, 0, 0, 0, 1, 1, 1],
            "twelve days back to six, maturing eight days ago"
        );
    }

    #[test]
    fn a_replay_is_kept_only_when_it_agrees_with_the_list_on_the_day_it_was_built() {
        let built = day_of(days_ago(3));
        let days = vec![
            (day_of(days_ago(4)), 2_500),
            (built.clone(), 2_544),
            (day_of(days_ago(2)), 2_559),
        ];
        assert_eq!(disagreement(&days, &built, 2_544), None, "the day it was built");
        let off = disagreement(&days, &built, 2_500).expect("a disagreement");
        assert!(off.contains("2544") && off.contains("2500"), "{off}");
        let missing = disagreement(&days, &day_of(days_ago(9)), 2_544).expect("a disagreement");
        assert!(missing.contains("does not reach"), "{missing}");
    }

    #[test]
    fn the_threshold_is_the_interval_it_says_and_not_a_day_less() {
        let below: Vec<u32> = counted(&mut [answer(10, 20)], 9, 8)
            .into_iter()
            .map(|(_, count)| count)
            .collect();
        assert_eq!(below, [0, 0], "twenty days is not mature at twenty-one");
        let at: Vec<u32> = counted(&mut [answer(10, 21)], 9, 8)
            .into_iter()
            .map(|(_, count)| count)
            .collect();
        assert_eq!(at, [1, 1], "twenty-one is");
    }

    #[test]
    fn a_lapse_takes_the_word_back_off_until_it_recovers() {
        let counts = counted(
            &mut [answer(10, 30), answer(6, 4), answer(3, 40)],
            11,
            1,
        );
        assert_eq!(
            counts.iter().map(|(_, count)| *count).collect::<Vec<_>>(),
            [0, 1, 1, 1, 1, 0, 0, 0, 1, 1, 1],
            "known from the tenth day back, lost on the sixth, back on the third"
        );
    }

    #[test]
    fn a_word_on_two_cards_counts_once_and_lasts_as_long_as_either() {
        let mut words = Answers::new();
        words.add(&["語".to_string()], &mut [answer(10, 30), answer(6, 2)], 21);
        words.add(&["語".to_string()], &mut [answer(8, 30)], 21);
        let days = days_until(&day_of(days_ago(11)), &day_of(days_ago(4)));
        let counts: Vec<u32> = words
            .daily_counts(&days)
            .into_iter()
            .map(|(_, count)| count)
            .collect();
        assert_eq!(counts, [0, 1, 1, 1, 1, 1, 1, 1], "the second card holds it up");
    }

    #[test]
    fn a_card_that_never_matured_carries_no_word_at_all() {
        let mut words = Answers::new();
        words.add(&["語".to_string()], &mut [answer(9, 1), answer(8, 10)], 21);
        assert_eq!(words.earliest(), None, "nothing it can speak for");
    }

    #[test]
    fn a_learning_step_reads_as_no_interval_at_all() {
        let log = serde_json::json!([
            { "id": 1_700_000_000_000_i64, "ivl": -600 },
            { "id": 1_700_100_000_000_i64, "ivl": 30 },
        ]);
        let answers = read_answers(&log).expect("a readable log");
        assert_eq!(answers[0].interval_days, 0, "negative seconds are not days");
        assert_eq!(answers[1].interval_days, 30);
    }

    #[test]
    fn an_answer_without_a_date_fails_the_replay_rather_than_moving_it() {
        let log = serde_json::json!([{ "ivl": 30 }]);
        assert!(read_answers(&log).is_err());
    }

    #[test]
    fn the_replay_reaches_back_no_further_than_ten_years() {
        let days = days_until(&day_of(days_ago(4_000)), &day_of(MIDDAY));
        assert_eq!(days.len(), MOST_DAYS);
        assert_eq!(
            days.last().map(|(day, _)| day.clone()),
            Some(day_of(MIDDAY)),
            "the newest days are the ones kept"
        );
    }
}
