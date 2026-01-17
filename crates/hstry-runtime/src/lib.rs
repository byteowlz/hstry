//! hstry-runtime: TypeScript adapter runtime
//!
//! This crate provides the runtime for executing TypeScript adapters
//! using Bun, Deno, or Node.js.

pub mod runner;

pub use runner::AdapterRunner;
pub use runner::Runtime;
