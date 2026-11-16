//! Shared lock logic. No operating-system calls.

mod name;

pub use name::{
    PRODUCT_DISPLAY_NAME, SERVICE_DISPLAY_NAME, SERVICE_ID, SERVICE_ID_PASCAL, pipe_name,
    registry_key, service_name,
};
