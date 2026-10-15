//! CrowDM transfer engine.
//!
//! * [`Engine`] owns jobs, the queue, the scheduler and persistence.
//! * [`transfer`] runs one download over one or more HTTP connections.
//! * [`store`] persists jobs, progress and settings in SQLite.
//!
//! See `docs/ARCHITECTURE.md` in the repository for the design.

pub mod checksum;
mod engine;
pub mod error;
pub mod file;
pub mod http;
pub mod media;
pub mod model;
pub mod names;
pub mod ratelimit;
pub mod settings;
pub mod store;
pub mod transfer;

pub use engine::{Engine, EngineEvent, JobPatch, ProbeRequest};
pub use error::{EngineError, TransferError};
pub use model::*;
pub use settings::Settings;
