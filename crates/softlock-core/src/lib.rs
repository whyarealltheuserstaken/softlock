//! Shared lock logic. No operating-system calls.

mod name;

pub use name::{INTERNAL_ID, PRODUCT_DISPLAY_NAME, pipe_name, registry_key, service_name};
