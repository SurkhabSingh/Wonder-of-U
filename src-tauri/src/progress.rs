//! Dated measurements of how much of the library the word list covers. Every number that
//! leaves here is a [`measured::Measured`], never a bare zero standing for "cannot tell".

// Remove with the sampler and command shim that will use it.
#[allow(dead_code)]
pub(crate) mod credit;
pub(crate) mod day;
pub(crate) mod library;
pub(crate) mod measured;
pub(crate) mod report;
pub(crate) mod store;
