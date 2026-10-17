//! C API shim over Open Cascade (OCCT) and its safe Rust wrapper.
//!
//! The only crate in the workspace allowed to contain `unsafe`. OCCT is linked
//! dynamically. The shim and `build.rs` arrive in M2.
