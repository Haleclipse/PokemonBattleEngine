//! Shared helpers for the integration test binaries. Each binary (`architecture`,
//! `oracle_framework`, `damage_oracle`) compiles this module separately and uses only part of
//! it, so "never used" here is a per-binary artefact rather than real dead code.
#![allow(dead_code, unused_imports)]

pub mod compare;
pub mod fixture;
pub mod pool;
pub mod projection;
pub mod scenario;
pub mod views;

pub use compare::*;
pub use fixture::*;
pub use projection::*;
pub use scenario::*;

pub use pool::*;
