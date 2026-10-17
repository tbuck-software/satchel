pub mod api;
pub mod registry;
#[cfg(desktop)]
pub mod runtime;
#[cfg(mobile)]
#[path = "runtime_mobile.rs"]
pub mod runtime;

// Export mobile fallback DTOs during desktop contract checks too.
#[cfg(all(test, feature = "ipc-types", desktop))]
pub mod runtime_mobile;
