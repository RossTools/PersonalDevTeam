//! Workflows (Phase 7a, docs/superpowers/specs/2026-10-20-phase-7a-engine-design.md): documents, data references,
//! conditions, validation, diffs, the event log's types and the API's records. Pure: pdt-store keeps them and pdt-api
//! runs them.

pub mod api;
pub mod cond;
pub mod diff;
pub mod doc;
pub mod events;
pub mod news;
pub mod refs;
pub mod tpl;
pub mod validate;
