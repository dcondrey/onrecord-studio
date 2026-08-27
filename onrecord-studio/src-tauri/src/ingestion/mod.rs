//! Secure portal discovery, traffic shaping, scheduling, and extraction pipelines.

pub mod client;
pub mod host_policy;
pub mod limiter;
pub mod queue;
// FEATURE(ingestion): Planned connector profiles make source coverage explicit without implying
// that every listed provider has a working adapter or permission to automate access.
pub mod source_catalog;

pub use client::ApiClient;
pub use host_policy::{HostGovernor, HostSnapshot, HostStopReason, RequestRefusal};
pub use limiter::RateLimiter;
pub use queue::QueueManager;
pub use source_catalog::{
    planned_source_profiles, AccessRequirement, ConnectorFamily, SourceProfile,
};
