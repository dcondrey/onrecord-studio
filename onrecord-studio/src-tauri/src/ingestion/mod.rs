//! Secure portal discovery, traffic shaping, scheduling, and extraction pipelines.

pub(crate) mod backoff;
pub mod client;
pub mod host_policy;
pub mod limiter;
pub mod queue;
pub mod source_catalog;

pub use client::ApiClient;
pub use host_policy::{HostGovernor, HostSnapshot, HostStopReason, RequestRefusal};
pub use limiter::RateLimiter;
pub use queue::QueueManager;
pub use source_catalog::{
    planned_source_profiles, AccessRequirement, AdapterAvailability, ConnectorFamily,
    ConnectorManifest, SourceProfile,
};
pub(crate) mod regional;
