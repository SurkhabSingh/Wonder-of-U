use serde::Serialize;

/// What is known about a number. Only `Unavailable` carries no value, and only its
/// constructor refuses to take one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MeasuredStatus {
    Known,
    Stale,
    Partial,
    Unavailable,
}

/// A number and whether it could be worked out. Fields are private and the constructors
/// are the only way in, so "known but empty" cannot be built.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Measured<T> {
    value: Option<T>,
    status: MeasuredStatus,
    as_of_ms: Option<u64>,
    reason: Option<String>,
}

impl<T> Measured<T> {
    pub(crate) fn known(value: T, as_of_ms: u64) -> Self {
        Self {
            value: Some(value),
            status: MeasuredStatus::Known,
            as_of_ms: Some(as_of_ms),
            reason: None,
        }
    }

    pub(crate) fn stale(value: T, as_of_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            value: Some(value),
            status: MeasuredStatus::Stale,
            as_of_ms: Some(as_of_ms),
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn partial(value: T, as_of_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            value: Some(value),
            status: MeasuredStatus::Partial,
            as_of_ms: Some(as_of_ms),
            reason: Some(reason.into()),
        }
    }

    pub(crate) fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            status: MeasuredStatus::Unavailable,
            as_of_ms: None,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Measured, MeasuredStatus};

    /// These names cross into TypeScript. A rename is silent there: the field reads
    /// `undefined` and every check against it quietly takes the wrong branch.
    #[test]
    fn the_wire_shape_is_what_the_frontend_reads() {
        let known = serde_json::to_value(Measured::known(42_u32, 1_700_000_000_000))
            .expect("a measured value must serialize");
        assert_eq!(known["value"], serde_json::json!(42));
        assert_eq!(known["status"], serde_json::json!("known"));
        assert_eq!(known["asOfMs"], serde_json::json!(1_700_000_000_000_u64));
        assert_eq!(known["reason"], serde_json::Value::Null);
    }

    /// Unavailable must arrive as explicit null, never zero and never an absent key:
    /// `undefined ?? 0` is the bug this type exists to prevent.
    #[test]
    fn an_unavailable_number_is_null_and_not_zero_and_not_absent() {
        let value = serde_json::to_value(Measured::<u32>::unavailable("Anki is not open."))
            .expect("a measured value must serialize");
        let object = value.as_object().expect("the payload is an object");

        assert!(object.contains_key("value"), "the key must be present: {value}");
        assert!(object.contains_key("asOfMs"), "the key must be present: {value}");
        assert_eq!(value["value"], serde_json::Value::Null);
        assert_eq!(value["asOfMs"], serde_json::Value::Null);
        assert_eq!(value["status"], serde_json::json!("unavailable"));
        assert_eq!(value["reason"], serde_json::json!("Anki is not open."));
    }

    #[test]
    fn the_two_qualified_states_keep_their_value() {
        let stale = serde_json::to_value(Measured::stale(7_u32, 100, "The word list changed."))
            .expect("a measured value must serialize");
        assert_eq!(stale["value"], serde_json::json!(7));
        assert_eq!(stale["status"], serde_json::json!("stale"));
        assert_eq!(stale["asOfMs"], serde_json::json!(100));

        let partial = serde_json::to_value(Measured::partial(7_u32, 100, "One file could not be read."))
            .expect("a measured value must serialize");
        assert_eq!(partial["value"], serde_json::json!(7));
        assert_eq!(partial["status"], serde_json::json!("partial"));
    }

    #[test]
    fn every_status_has_a_distinct_wire_name() {
        let names: Vec<String> = [
            MeasuredStatus::Known,
            MeasuredStatus::Stale,
            MeasuredStatus::Partial,
            MeasuredStatus::Unavailable,
        ]
        .iter()
        .map(|status| serde_json::to_string(status).expect("a status must serialize"))
        .collect();

        assert_eq!(
            names,
            vec![
                "\"known\"".to_string(),
                "\"stale\"".to_string(),
                "\"partial\"".to_string(),
                "\"unavailable\"".to_string()
            ]
        );
    }
}
