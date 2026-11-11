//! The platform bridges. Each target's bridge returns its backend when implemented; the selectors
//! chain all five and fall back to `UnsupportedBackend` (central) or to none (server).

mod android;
mod apple;
mod linux;
mod web;
mod windows;

use std::sync::Arc;

use crate::backend::{CentralBackend, ServerBackend};
use crate::unsupported::UnsupportedBackend;

/// The central backend of the target this build runs on, with the shared fallback for targets
/// whose bridge is absent or not implemented.
pub(crate) fn central_backend() -> Arc<dyn CentralBackend> {
    apple::central_backend()
        .or_else(android::central_backend)
        .or_else(windows::central_backend)
        .or_else(linux::central_backend)
        .or_else(web::central_backend)
        .unwrap_or_else(|| Arc::new(UnsupportedBackend))
}

/// The server backend of the target this build runs on, when a bridge provides one.
pub(crate) fn server_backend() -> Option<Arc<dyn ServerBackend>> {
    apple::server_backend()
        .or_else(android::server_backend)
        .or_else(windows::server_backend)
        .or_else(linux::server_backend)
        .or_else(web::server_backend)
}
