//! Plugin packages: manifests, the store and its staging, archives,
//! validation, the package manager and the secret index.

pub(crate) mod manifest;
pub(crate) mod package_archive;
pub(crate) mod package_manager;
pub mod package_stage;
pub mod package_store;
pub(crate) mod package_validate;
pub(crate) mod safe_remove;
pub(crate) mod secret_index;
