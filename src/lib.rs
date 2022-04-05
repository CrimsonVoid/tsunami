#![feature(
    let_else,
    try_blocks,
    label_break_value,
    // async_closure,
    type_ascription,
    crate_visibility_modifier,
    let_chains,
)]
#![feature(
    io_slice_advance,
    iterator_try_collect,
    iter_collect_into,
    mixed_integer_ops,
    default_free_fn
)]

mod error;
mod torrent_ast;
mod utils;

#[allow(dead_code)]
mod peer;
#[allow(dead_code)]
mod torrent;
#[allow(dead_code)]
pub mod tsunami;
