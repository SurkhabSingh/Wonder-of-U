use std::collections::{HashMap, HashSet};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::app_types::{
    KnownWordsBuild, KnownWordsState, SharedPersistedState, VocabularySource,
};
use crate::progress::day::DayKey;
use crate::progress::vocabulary::{self, WordHistory};

use super::client::{
    anki_connect_health_check, anki_find_cards, anki_find_notes, anki_notes_info,
    anki_offline_message, anki_reviews_of_cards, json_array,
};
use super::known_words::{note_expression, note_type_query, note_type_search};

const REVIEW_LOG_BATCH: usize = 1_000;
const NOTES_INFO_BATCH: usize = 500;

/// A review older than this is a broken timestamp rather than a memory, and the day list it
/// would ask for is measured in decades.
const MOST_DAYS: usize = 3_660;

// What an answer was, as the review log numbers it.
const REVIEW: i64 = 1;
const RELEARNING: i64 = 2;
const FILTERED: i64 = 3;
const MANUAL: i64 = 4;
const RESCHEDULED: i64 = 5;

/// One row of a card's log. Intervals are days, or negative seconds for a (re)learning step.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Answer {
    pub(crate) at_ms: i64,
    pub(crate) kind: i64,
    pub(crate) interval: i64,
    pub(crate) before: i64,
}

/// From when a stretch began until the moment it stopped holding. The interval a card carries
/// now has not stopped, so its stretch runs to the end of time.
type Span = (i64, i64);

fn relearning(answer: &Answer) -> bool {
    answer.interval <= 0 && matches!(answer.kind, REVIEW | RELEARNING)
}

/// The interval in days each answer left its card on, oldest first. A preview in a filtered
/// deck changes nothing, and a relearning card keeps the interval its lapse gave it.
fn settled(answers: &mut [Answer], mature_now: bool, mature_after_days: i64) -> Vec<(i64, i64)> {
    answers.sort_by_key(|answer| answer.at_ms);
    let rows: Vec<Answer> = answers
        .iter()
        .filter(|answer| !(answer.kind == FILTERED && answer.interval <= 0))
        .copied()
        .collect();
    let mut settled = Vec::with_capacity(rows.len());
    let mut at = 0;
    while at < rows.len() {
        if !relearning(&rows[at]) {
            settled.push((rows[at].at_ms, rows[at].interval.max(0)));
            at += 1;
            continue;
        }
        let end = rows[at..]
            .iter()
            .position(|answer| !relearning(answer))
            .map_or(rows.len(), |steps| at + steps);
        // The log names that interval only once the card graduates or is rescheduled by hand;
        // until then only the card itself carries it.
        let held = match rows.get(end) {
            Some(next) if next.kind == RELEARNING => next.interval,
            Some(next) if matches!(next.kind, MANUAL | RESCHEDULED) => next.before,
            Some(_) => 0,
            None if mature_now => mature_after_days,
            None => 0,
        };
        settled.extend(rows[at..end].iter().map(|answer| (answer.at_ms, held.max(0))));
        at = end;
    }
    settled
}

fn mature_spans(settled: &[(i64, i64)], mature_after_days: i64) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for (at, &(from, interval)) in settled.iter().enumerate() {
        if interval < mature_after_days {
            continue;
        }
        let until = settled.get(at + 1).map_or(i64::MAX, |next| next.0);
        match spans.last_mut() {
            Some(last) if last.1 == from => last.1 = until,
            _ => spans.push((from, until)),
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

/// The stretches each word stood known, folded in a card at a time and keeping no reviews.
pub(crate) struct Answers {
    by_word: HashMap<String, Vec<Span>>,
}

impl Answers {
    pub(crate) fn new() -> Self {
        Self {
            by_word: HashMap::new(),
        }
    }

    pub(crate) fn add(
        &mut self,
        word: &str,
        answers: &mut [Answer],
        mature_now: bool,
        mature_after_days: u32,
    ) {
        let threshold = i64::from(mature_after_days);
        let spans = mature_spans(&settled(answers, mature_now, threshold), threshold);
        if !spans.is_empty() {
            self.by_word.entry(word.to_string()).or_default().extend(spans);
        }
    }

    pub(crate) fn known_at(&self, moment_ms: i64) -> HashSet<String> {
        self.by_word
            .iter()
            .filter(|(_, spans)| {
                spans
                    .iter()
                    .any(|&(from, to)| from <= moment_ms && moment_ms < to)
            })
            .map(|(word, _)| word.clone())
            .collect()
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WordHistorySnapshot {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) days: usize,
}

/// Keeps a replay only when it holds the very words the list held when it was built. One that
/// disagrees has misread the log, and a wrong history is worse than none.
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
            .map(|index| (index.built_at_ms, index.words.clone()))
    };
    let Some((built_at_ms, live_words)) = live else {
        return Ok(WordHistorySnapshot {
            status: "unbuilt".into(),
            message: "Refresh your word list first, so the history can be checked against it."
                .into(),
            days: 0,
        });
    };
    let built_on = crate::progress::day::day_key_for_ms(built_at_ms)
        .ok_or_else(|| "Your word list carries a date outside the calendar.".to_string())?;
    let built_at = i64::try_from(built_at_ms)
        .map_err(|_| "Your word list carries a date outside the calendar.".to_string())?;

    let today = crate::progress::day::today();
    let replayed = replay(&build, &today, built_at)?;
    if let Some(message) = disagreement(&replayed.known_when_built, &live_words, &built_on) {
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

fn disagreement(
    replayed: &HashSet<String>,
    live: &HashSet<String>,
    built_on: &DayKey,
) -> Option<String> {
    let mut differ: Vec<&str> = replayed
        .symmetric_difference(live)
        .map(String::as_str)
        .collect();
    differ.sort_unstable();
    let count = differ.len();
    let named = match differ[..] {
        [] => return None,
        [one] => format!("one word, {one}"),
        [first, second] => format!("{count} words, {first} and {second}"),
        [first, second, third] => format!("{count} words, {first}, {second} and {third}"),
        [first, second, third, ..] => {
            format!("{count} words, among them {first}, {second} and {third}")
        }
    };
    Some(format!(
        "The review log replays to {} words when your list was built on {built_on}, and the list holds {}. They disagree about {named}. Nothing was kept. Refresh your word list, then try again.",
        replayed.len(),
        live.len()
    ))
}

pub(crate) struct Replayed {
    pub(crate) days: Vec<(DayKey, u32)>,
    pub(crate) words_today: u32,
    pub(crate) known_when_built: HashSet<String>,
    pub(crate) cards: usize,
}

/// One failed batch fails the whole replay: a history missing a note type is not a shorter
/// history, it is a collapse the chart would draw as forgetting.
pub(crate) fn replay(
    build: &KnownWordsBuild,
    today: &DayKey,
    built_at_ms: i64,
) -> Result<Replayed, String> {
    let mut answers = Answers::new();
    let mut cards = 0_usize;
    for source in &build.sources {
        cards += read_source(source, build.mature_after_days, &mut answers)?;
    }
    let known_when_built = answers.known_at(built_at_ms);
    let Some(earliest) = answers.earliest() else {
        return Ok(Replayed {
            days: Vec::new(),
            words_today: 0,
            known_when_built,
            cards,
        });
    };
    let first = crate::progress::day::day_key_for_ms(u64::try_from(earliest).unwrap_or(0))
        .ok_or_else(|| "An answer in the review log is dated outside the calendar.".to_string())?;
    let days = answers.daily_counts(&days_until(&first, today));
    Ok(Replayed {
        words_today: days.last().map_or(0, |(_, count)| *count),
        known_when_built,
        days,
        cards,
    })
}

fn read_source(
    source: &VocabularySource,
    mature_after_days: u32,
    answers: &mut Answers,
) -> Result<usize, String> {
    let note_ids = anki_find_notes(&note_type_search(&source.note_type))?;
    if note_ids.is_empty() {
        return Ok(0);
    }
    let word_of_card = read_words(&note_ids, &source.field)?;
    let mature_now: HashSet<i64> =
        anki_find_cards(&note_type_query(&source.note_type, mature_after_days))?
            .into_iter()
            .collect();
    let mut cards: Vec<(i64, &str)> = word_of_card
        .iter()
        .map(|(card, word)| (*card, word.as_str()))
        .collect();
    cards.sort_unstable();

    for batch in cards.chunks(REVIEW_LOG_BATCH) {
        let ids: Vec<i64> = batch.iter().map(|(card, _)| *card).collect();
        let logs = anki_reviews_of_cards(&ids)?;
        fold_logs(batch, &logs, &mature_now, mature_after_days, answers)?;
    }
    Ok(cards.len())
}

/// A card the reply leaves out would read as one never answered, so it fails the replay.
fn fold_logs(
    batch: &[(i64, &str)],
    logs: &serde_json::Value,
    mature_now: &HashSet<i64>,
    mature_after_days: u32,
    answers: &mut Answers,
) -> Result<(), String> {
    let logs = logs
        .as_object()
        .ok_or_else(|| "Anki's review log was not a list per card.".to_string())?;
    for &(card, word) in batch {
        let log = logs
            .get(&card.to_string())
            .ok_or_else(|| "Anki's review log left out a card it was asked about.".to_string())?;
        let mut given = read_answers(log)?;
        answers.add(word, &mut given, mature_now.contains(&card), mature_after_days);
    }
    Ok(())
}

/// `cardsToNotes` answers with distinct notes in no order, so each note's own card list is
/// what pairs a card with its word.
fn read_words(note_ids: &[i64], field: &str) -> Result<HashMap<i64, String>, String> {
    let mut words = HashMap::new();
    for batch in note_ids.chunks(NOTES_INFO_BATCH) {
        let notes = anki_notes_info(batch)?;
        words_of_cards(json_array(&notes, "note list")?, field, &mut words)?;
    }
    Ok(words)
}

fn words_of_cards(
    notes: &[serde_json::Value],
    field: &str,
    words: &mut HashMap<i64, String>,
) -> Result<(), String> {
    for note in notes {
        let Some(word) = note_expression(note, field) else {
            continue;
        };
        let cards = note
            .get("cards")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "Anki answered with a note that does not list its cards.".to_string())?;
        for card in cards.iter().filter_map(serde_json::Value::as_i64) {
            words.insert(card, word.clone());
        }
    }
    Ok(())
}

/// A row missing any of its numbers fails the replay; a guess at it would move the history.
fn read_answers(log: &serde_json::Value) -> Result<Vec<Answer>, String> {
    json_array(log, "one card's review log")?
        .iter()
        .map(|review| {
            let number = |name: &str| {
                review.get(name).and_then(serde_json::Value::as_i64).ok_or_else(|| {
                    "Anki's review log has an answer this app cannot read.".to_string()
                })
            };
            Ok(Answer {
                at_ms: number("id")?,
                kind: number("type")?,
                interval: number("ivl")?,
                before: number("lastIvl")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::day::day_key_for_ms;

    const DAY_MS: i64 = 86_400_000;
    const LEARNING: i64 = 0;

    /// Noon UTC on 10 September.
    const MIDDAY: i64 = 1_789_041_600_000;

    fn days_ago(days: i64) -> i64 {
        MIDDAY - days * DAY_MS
    }

    fn day_of(ms: i64) -> DayKey {
        day_key_for_ms(u64::try_from(ms).expect("a positive timestamp")).expect("a day")
    }

    fn row(days_before: i64, interval: i64, kind: i64) -> Answer {
        Answer {
            at_ms: days_ago(days_before),
            kind,
            interval,
            before: 0,
        }
    }

    fn answer(days_before: i64, interval: i64) -> Answer {
        row(days_before, interval, REVIEW)
    }

    fn known_days(answers: &mut [Answer], mature_now: bool, first: i64, last: i64) -> Vec<u32> {
        let mut words = Answers::new();
        words.add("語", answers, mature_now, 21);
        let days = days_until(&day_of(days_ago(first)), &day_of(days_ago(last)));
        words
            .daily_counts(&days)
            .into_iter()
            .map(|(_, count)| count)
            .collect()
    }

    fn set(words: &[&str]) -> HashSet<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    #[test]
    fn a_word_counts_from_the_day_its_card_matured() {
        assert_eq!(
            known_days(&mut [answer(10, 3), answer(8, 25)], false, 12, 6),
            [0, 0, 0, 0, 1, 1, 1],
            "twelve days back to six, maturing eight days ago"
        );
    }

    fn note(id: i64, word: &str, cards: serde_json::Value) -> serde_json::Value {
        let mut note = serde_json::json!({
            "noteId": id,
            "fields": { "Word": { "value": word, "order": 0 } },
        });
        if !cards.is_null() {
            note["cards"] = cards;
        }
        note
    }

    #[test]
    fn every_card_of_a_note_carries_its_word() {
        let notes = [
            note(1, "語", serde_json::json!([11, 12])),
            note(2, "本", serde_json::json!([21])),
            note(3, "", serde_json::json!([31])),
        ];
        let mut words = HashMap::new();
        words_of_cards(&notes, "Word", &mut words).expect("readable notes");
        assert_eq!(words.get(&11).map(String::as_str), Some("語"));
        assert_eq!(words.get(&12).map(String::as_str), Some("語"), "the second card too");
        assert_eq!(words.get(&21).map(String::as_str), Some("本"));
        assert!(!words.contains_key(&31), "an empty field carries no word");
    }

    #[test]
    fn a_note_that_does_not_list_its_cards_fails_the_replay() {
        let mut words = HashMap::new();
        let notes = [note(1, "語", serde_json::Value::Null)];
        assert!(words_of_cards(&notes, "Word", &mut words).is_err());
    }

    #[test]
    fn a_card_the_reply_leaves_out_fails_the_replay() {
        let logs = serde_json::json!({
            "11": [{ "id": days_ago(9), "type": REVIEW, "ivl": 30, "lastIvl": 10 }],
        });
        let mut answers = Answers::new();
        let whole = fold_logs(&[(11, "語")], &logs, &HashSet::new(), 21, &mut answers);
        assert!(whole.is_ok());
        let short = fold_logs(&[(11, "語"), (12, "本")], &logs, &HashSet::new(), 21, &mut answers);
        assert!(short.is_err(), "card 12 is not in the reply");
    }

    #[test]
    fn a_replay_is_kept_only_when_it_holds_the_words_the_list_held() {
        let built = day_of(days_ago(3));
        assert_eq!(disagreement(&set(&["語", "本"]), &set(&["本", "語"]), &built), None);
        let swapped = disagreement(&set(&["語", "本"]), &set(&["語", "猫"]), &built)
            .expect("the same count, other words");
        assert!(swapped.contains("本") && swapped.contains("猫"), "{swapped}");
        assert!(swapped.contains("Refresh your word list"), "it says what settles it");
        let short = disagreement(&set(&["語"]), &set(&["語", "本"]), &built).expect("one missing");
        assert!(short.contains("one word, 本"), "{short}");
    }

    #[test]
    fn the_list_is_compared_at_the_moment_it_was_built_not_the_end_of_that_day() {
        let mut words = Answers::new();
        words.add("語", &mut [answer(3, 30)], false, 21);
        let built = days_ago(3) - 3_600_000;
        assert!(words.known_at(built).is_empty(), "an hour before the card matured");
        assert_eq!(words.known_at(days_ago(3)), set(&["語"]), "the moment it did");
        assert_eq!(
            words.daily_counts(&days_until(&day_of(built), &day_of(built)))[0].1,
            1,
            "while the day it was built ends with the word known"
        );
    }

    #[test]
    fn the_threshold_is_the_interval_it_says_and_not_a_day_less() {
        assert_eq!(known_days(&mut [answer(10, 20)], false, 9, 8), [0, 0]);
        assert_eq!(known_days(&mut [answer(10, 21)], false, 9, 8), [1, 1]);
    }

    #[test]
    fn a_lapse_takes_the_word_back_off_until_it_recovers() {
        assert_eq!(
            known_days(&mut [answer(10, 30), answer(6, 4), answer(3, 40)], false, 11, 1),
            [0, 1, 1, 1, 1, 0, 0, 0, 1, 1, 1],
            "known from the tenth day back, lost on the sixth, back on the third"
        );
    }

    #[test]
    fn a_preview_in_a_filtered_deck_leaves_the_interval_where_it_was() {
        let mut rows = [answer(10, 30), row(6, -600, FILTERED), row(5, 0, FILTERED)];
        assert_eq!(known_days(&mut rows, false, 11, 3), [0, 1, 1, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn a_filtered_review_that_set_an_interval_counts() {
        let mut rows = [answer(10, 5), row(6, 30, FILTERED)];
        assert_eq!(known_days(&mut rows, false, 11, 4), [0, 0, 0, 0, 0, 1, 1, 1]);
    }

    #[test]
    fn a_relearning_card_keeps_the_interval_its_lapse_gave_it() {
        let mut kept = [answer(10, 30), answer(6, -600), row(5, 25, RELEARNING)];
        assert_eq!(
            known_days(&mut kept, false, 11, 3),
            [0, 1, 1, 1, 1, 1, 1, 1, 1],
            "the lapse left it at twenty-five days"
        );
        let mut reset = [answer(10, 30), answer(6, -600), row(5, 1, RELEARNING)];
        assert_eq!(
            known_days(&mut reset, false, 11, 3),
            [0, 1, 1, 1, 1, 0, 0, 0, 0],
            "the lapse sent it back to one day"
        );
    }

    #[test]
    fn a_reschedule_by_hand_names_the_interval_a_relearning_card_held() {
        let forgot = Answer {
            before: 25,
            ..row(4, 0, MANUAL)
        };
        let mut rows = [answer(10, 30), answer(6, -600), forgot];
        assert_eq!(known_days(&mut rows, false, 11, 2), [0, 1, 1, 1, 1, 1, 1, 0, 0, 0]);
    }

    #[test]
    fn a_card_still_relearning_is_known_by_the_interval_it_holds_now() {
        let mut rows = [answer(10, 30), answer(6, -600)];
        assert_eq!(known_days(&mut rows, true, 11, 3), [0, 1, 1, 1, 1, 1, 1, 1, 1]);
        let mut rows = [answer(10, 30), answer(6, -600)];
        assert_eq!(known_days(&mut rows, false, 11, 3), [0, 1, 1, 1, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn a_new_card_learning_its_steps_is_worth_nothing() {
        let mut words = Answers::new();
        words.add("語", &mut [row(9, -600, LEARNING)], true, 21);
        assert_eq!(words.earliest(), None, "a first step is not a lapse");
    }

    #[test]
    fn a_word_on_two_cards_counts_once_and_lasts_as_long_as_either() {
        let mut words = Answers::new();
        words.add("語", &mut [answer(10, 30), answer(6, 2)], false, 21);
        words.add("語", &mut [answer(8, 30)], false, 21);
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
        words.add("語", &mut [answer(9, 1), answer(8, 10)], false, 21);
        assert_eq!(words.earliest(), None, "nothing it can speak for");
    }

    #[test]
    fn an_answer_reads_the_four_numbers_anki_logs() {
        let log = serde_json::json!([
            { "id": 1_700_000_000_000_i64, "type": RELEARNING, "ivl": 25, "lastIvl": -600 },
        ]);
        let read = read_answers(&log).expect("a readable log");
        assert_eq!(
            (read[0].at_ms, read[0].kind, read[0].interval, read[0].before),
            (1_700_000_000_000, RELEARNING, 25, -600)
        );
    }

    #[test]
    fn an_answer_missing_a_number_fails_the_replay_rather_than_moving_it() {
        let whole = serde_json::json!({ "id": 1_i64, "type": 1, "ivl": 30, "lastIvl": 10 });
        for missing in ["id", "type", "ivl", "lastIvl"] {
            let mut review = whole.clone();
            review.as_object_mut().expect("an object").remove(missing);
            assert!(read_answers(&serde_json::json!([review])).is_err(), "without {missing}");
        }
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
