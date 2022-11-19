#![feature(
    let_chains,
    try_blocks,
    type_ascription,
    io_slice_advance,
    iterator_try_collect
)]

mod error;
mod torrent_ast;
#[allow(dead_code)]
mod utils;

#[allow(dead_code)]
mod peer;
#[allow(dead_code)]
mod torrent;
#[allow(dead_code)]
pub mod tsunami;

#[cfg(test)]
mod tests;
