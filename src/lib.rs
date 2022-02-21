#![feature(
    let_else,
    try_blocks,
    label_break_value,
    // async_closure,
    type_ascription,
    in_band_lifetimes,
    crate_visibility_modifier,
)]
#![feature(io_slice_advance)]

mod bencode;
mod error;
mod torrent;
mod utils;

#[allow(dead_code)]
mod peer;
#[allow(dead_code)]
pub mod tsunami;
