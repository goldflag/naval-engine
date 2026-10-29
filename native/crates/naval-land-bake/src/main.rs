//! `naval-land-bake --data <folder> make|bake …`: a land release made from its pinned sources, and its squares baked
//! into the store under a data folder (`<data>/land`, the sources at `<data>/land-sources`), without the game. The
//! commands and their flags are `naval-client land-release`'s (`naval_land_bake::cli`); a game's data folder is the
//! user's (`~/Library/Application Support/gg.tomato.naval` on macOS), any other folder keeps a store of its own.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "naval-land-bake",
    about = "Make a land release and bake its squares, without the game"
)]
struct Args {
    /// The data folder the store (`land`) and the sources (`land-sources`) are under.
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    command: naval_land_bake::cli::Command,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let Some(data) = args.data else {
        bail!(
            "--data <folder> names the data folder (the store is <folder>/land, the sources <folder>/land-sources)"
        )
    };
    naval_land_bake::cli::run(args.command, &data)
}
