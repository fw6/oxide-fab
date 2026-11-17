//! Persistence for oxide-fab: the PostgreSQL event store, its transactional
//! outbox, and the `GreptimeDB` writer that consumes it.

pub mod greptime;
pub mod outbox;
pub mod pg;

pub use greptime::{GreptimeDb, GreptimeError, SpcBucket};
pub use outbox::{OutboxSample, OutboxTimeline, Publisher, ReplayReport};
pub use pg::{DbCounts, LotGenealogy, OutboxStats, PersistedEvent, PgStore, StoreError};
