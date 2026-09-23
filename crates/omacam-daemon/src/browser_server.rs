//! Browser camera server entry point; implementation lives in focused modules.

#[path = "browser/mod.rs"]
mod browser;

pub(crate) use browser::{BrowserServeOptions, run};
