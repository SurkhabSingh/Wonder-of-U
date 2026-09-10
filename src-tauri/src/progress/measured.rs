use serde::Serialize;

/// What is known about a number, beyond the number itself.
///
/// Four states rather than a flag, because three of them still carry a value and a reader
/// needs different words for each. `Unavailable` is the only one that carries nothing,
/// and it is the only one whose constructor refuses to take a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MeasuredStatus {
    /// Worked out from complete inputs.
    Known,
    /// A real measurement, taken under settings that have since changed. Shown with its
    /// date rather than withheld: the last true answer beats no answer, so long as it is
    /// dated and said to be old.
    Stale,
    /// Worked out, but an input could not be read. The value is real and smaller than the
    /// truth, so the shortfall is named rather than the number hidden.
    Partial,
    /// Not worked out at all.
    Unavailable,
}

/// A number, together with whether the app could actually work it out.
///
/// The whole feature turns on one distinction: "the answer is zero" and "there is no
/// answer" must never reach the reader looking the same. A bare number cannot hold that
/// difference, so no number crosses to the frontend bare.
///
/// The fields are private and the constructors are the only way in, so a value claiming to
/// be known while carrying nothing cannot be built. `unavailable` takes no value rather
/// than an ignored one — that is what makes the accidental zero impossible instead of
/// merely discouraged.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Measured<T> {
    value: Option<T>,
    status: MeasuredStatus,
    /// When the value was measured. `None` exactly when there is no value.
    as_of_ms: Option<u64>,
    /// One sentence for the reader, on every status but `Known`.
    reason: Option<String>,
}

impl<T> Measured<T> {
    /// A complete answer, measured at `as_of_ms`.
    pub(crate) fn known(value: T, as_of_ms: u64) -> Self {
        Self {
            value: Some(value),
            status: MeasuredStatus::Known,
            as_of_ms: Some(as_of_ms),
            reason: None,
        }
    }

    /// A real answer measured under inputs that have since changed.
    pub(crate) fn stale(value: T, as_of_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            value: Some(value),
            status: MeasuredStatus::Stale,
            as_of_ms: Some(as_of_ms),
            reason: Some(reason.into()),
        }
    }

    /// A real answer built from less than everything it should have read.
    pub(crate) fn partial(value: T, as_of_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            value: Some(value),
            status: MeasuredStatus::Partial,
            as_of_ms: Some(as_of_ms),
            reason: Some(reason.into()),
        }
    }

    /// No answer. Takes no value, so there is nothing for a caller to pass as a
    /// placeholder and nothing for a renderer to mistake for a measurement.
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

    /// These names cross into TypeScript, where a renderer decides between a number and a
    /// dash by reading them. A rename here is silent on the other side: the field reads
    /// `undefined`, every check against it fails, and the page settles on whichever branch
    /// `undefined` happens to take — which looks like working software.
    #[test]
    fn the_wire_shape_is_what_the_frontend_reads() {
        let known = serde_json::to_value(Measured::known(42_u32, 1_700_000_000_000))
            .expect("a measured value must serialize");
        assert_eq!(known["value"], serde_json::json!(42));
        assert_eq!(known["status"], serde_json::json!("known"));
        assert_eq!(known["asOfMs"], serde_json::json!(1_700_000_000_000_u64));
        assert_eq!(known["reason"], serde_json::Value::Null);
    }

    /// The one that matters. An unavailable number must arrive as an explicit null under
    /// both keys — not as zero, and not as a key the frontend never sees, because a
    /// missing key reads as `undefined` and `undefined ?? 0` is the bug this type exists
    /// to prevent.
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

    /// Stale and partial both keep their value on purpose: a dated answer is useful and a
    /// withheld one is not. Only the words around them change.
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
