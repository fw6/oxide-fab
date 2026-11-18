//! The five domain panels. Each public `panel` returns the cards of one
//! `SmartFactory` domain; the cards read signals inside `dynamic` closures so
//! they follow the poll without rebuilding the page.
//!
//! A card is one linear function on purpose: splitting a 100-line card into
//! helpers hides the data flow it is meant to make reviewable.
#![allow(clippy::too_many_lines)]

pub mod cross;
pub mod mes;
pub mod mod_helpers;
pub mod productivity;
pub mod quality;
pub mod supply;

pub use mod_helpers::*;
