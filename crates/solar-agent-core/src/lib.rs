//! Provider- and UI-independent contracts for Solar agent runtimes.
//!
//! This crate is the stable boundary between Solar application services and
//! concrete runtimes such as Rig-backed providers or ACP agents. It must not
//! depend on desktop UI, provider SDKs, persistence, or application wire protocols.

mod provider_run;
mod runtime;
mod task_list;
mod tool_policy;
mod types;

pub use provider_run::*;
pub use runtime::*;
pub use task_list::*;
pub use tool_policy::*;
pub use types::*;
