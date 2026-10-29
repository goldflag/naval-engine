//! `naval-land synthetic <land folder>`: installs the synthetic release, every square of it, into the store at a
//! folder (a game's `<data>/land`, or any other) and prints its id. Running it twice changes nothing.

use naval_land::{Store, install_synthetic};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, folder] = args.as_slice() else {
        eprintln!("usage: naval-land synthetic <land folder>");
        std::process::exit(2);
    };
    if command != "synthetic" {
        eprintln!("unknown command {command}; usage: naval-land synthetic <land folder>");
        std::process::exit(2);
    }
    let result = Store::open(folder, u64::MAX).and_then(|store| {
        let id = install_synthetic(&store)?;
        Ok((id, store.usage()))
    });
    match result {
        Ok((id, usage)) => println!(
            "{} {id}: {} squares, {} bytes, in {folder}",
            naval_land::format::synthetic::NAME,
            usage.squares,
            usage.bytes
        ),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
