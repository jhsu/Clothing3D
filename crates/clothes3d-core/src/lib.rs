//! Platform-independent core of clothes3d: the pattern data model and free
//! sewing logic. No I/O or UI dependencies, so it builds for native and
//! `wasm32-unknown-unknown` alike.

pub mod curves;
pub mod dxf;
pub mod model;
pub mod sewing;
