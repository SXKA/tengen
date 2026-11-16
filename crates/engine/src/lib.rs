//! Tengen: a freestyle Gomoku engine.

pub mod bench;
pub mod board;
pub mod eval;
pub mod movegen;
pub mod pattern;
pub mod search;
pub mod tt;

pub use board::{Board, Color, Pos};
pub use search::{Engine, Info, Limits, SearchResult};
