#![feature(let_chains, try_blocks, iterator_try_collect, test)]

mod error;
#[allow(non_snake_case)]
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

#[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
compile_error!("only 32bit or 64bit systems supported");
