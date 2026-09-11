//! What the app can say about a learner's progress, and what it must refuse to say.
//!
//! The module stores as little as it can. Cards in Anki already carry their own creation
//! time, and the recording history already carries when every recording arrived, so
//! neither is copied here — a lost progress file costs only the things that genuinely
//! cannot be worked out again.
//!
//! What it does store is what nothing else records: dated measurements of how much of the
//! library the word list covers. Every number that leaves here is a [`measured::Measured`],
//! which carries whether it could be worked out at all, because the one thing this feature
//! must never do is show a zero where the honest answer is "I could not tell".

// Nothing calls this yet; the sampler and the command shim are the next increment.
// Delete this attribute when they land — it must not outlive them.
#[allow(dead_code)]
pub(crate) mod credit;
pub(crate) mod day;
pub(crate) mod library;
pub(crate) mod measured;
pub(crate) mod report;
pub(crate) mod store;
