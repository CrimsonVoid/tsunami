#![feature(
    let_else,
    let_chains,
    try_blocks,
    type_ascription,
    crate_visibility_modifier,
    // label_break_value,
    // async_closure,
)]
#![feature(io_slice_advance, iterator_try_collect)]

mod error;
mod torrent_ast;
#[allow(dead_code)]
mod utils;

#[allow(dead_code, irrefutable_let_patterns)]
mod peer;
#[allow(dead_code)]
mod torrent;
#[allow(dead_code)]
pub mod tsunami;
