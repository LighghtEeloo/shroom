//! A typed client for a separately installed, local Lume HTTP service.
//!
//! This crate never invokes the Lume CLI. It exposes VM operations, not Shroom's
//! authenticated SSH workspace contract: upstream currently lacks the guest
//! administration and network controls needed to implement that contract.
//! HTTP acceptance is not VM readiness, and VM readiness is not SSH readiness.
//!
//! Storage is explicitly scoped on every request. Use a dedicated Lume service
//! and storage directory: upstream's running-VM cache is keyed by name alone.
//! Request cancellation does not roll back work accepted by that service.

mod client;
mod model;

pub use client::{Client, Config};
pub use model::{
    Accepted, CreateVm, DiskSize, Guest, GuestOs, SharedDirectory, Vm, VmName, VmState,
};

/// Upstream source revision used to verify the HTTP schemas and semantics.
pub const REVIEWED_REVISION: &str = "9bbfa7dd3e27ca7f1861ede70aaca390174493f9";

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    List,
    Get,
    Create,
    Clone,
    Start,
    ForceStop,
    ForceDelete,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Lume configuration: {0}")]
    InvalidConfig(&'static str),
    #[error(
        "VM name must be a lowercase ASCII slug of 1–48 characters, starting with a letter or digit"
    )]
    InvalidName,
    #[error("invalid VM creation request: {0}")]
    InvalidCreate(&'static str),
    #[error("failed to configure the Lume HTTP client: {0}")]
    HttpClient(#[source] reqwest::Error),
    #[error("Lume {operation:?} transport failed: {source}")]
    Transport {
        operation: Operation,
        #[source]
        source: reqwest::Error,
    },
    #[error("Lume {operation:?} returned HTTP {status}: {message}")]
    Api {
        operation: Operation,
        status: u16,
        message: String,
    },
    #[error("Lume {operation:?} returned an invalid response: {source}")]
    Decode {
        operation: Operation,
        #[source]
        source: serde_json::Error,
    },
    #[error("Lume {operation:?} response exceeds the {limit}-byte limit")]
    ResponseTooLarge { operation: Operation, limit: usize },
    #[error("Lume {operation:?} exceeded the request deadline")]
    Timeout { operation: Operation },
    #[error("Lume {operation:?} returned a different VM name than requested")]
    IdentityMismatch { operation: Operation },
}
