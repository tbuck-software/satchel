//! Mail merge: one personalised copy of a message per recipient of a list.
//!
//! - [`csv`] reads recipient lists the way spreadsheets export them.
//! - [`template`] fills `{{Field}}` and `{{Field|fallback}}` placeholders.
//! - [`plan`] decides which rows can be sent and renders each one.
//! - [`job`] stores a job and moves it through the outbox, a message at a time.

pub mod csv;
pub mod job;
pub mod plan;
pub mod template;
