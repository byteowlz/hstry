//! hstry-runtime: TypeScript adapter runtime
//!
//! This crate provides the runtime for executing TypeScript adapters
//! using Bun, Deno, or Node.js.

pub mod runner;

pub use runner::AdapterRequest;
pub use runner::AdapterResponse;
pub use runner::AdapterRunner;
pub use runner::ExportConversation;
pub use runner::ExportOptions;
pub use runner::ExportResult;
pub use runner::ParsedMessage;
pub use runner::Runtime;
pub use runner::RuntimeKind;
