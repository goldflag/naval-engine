//! `naval-vessel inspect <model.glb|model.gltf> [--write]`: what a downloaded model is (its bounds, a guess at its
//! axes, its materials, triangles and textures), and with `--write` a starter setup file beside it
//! (`<model>.vessel.json`, never over one that is there) to edit: her real length, her draft, her funnels and lamps.
//!
//!     bun run cargo -- run -p naval-vessel -- inspect ~/Downloads/destroyer.glb --write

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    use std::path::Path;

    use anyhow::{Context, bail};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some("inspect"), Some(path)) = (args.first().map(String::as_str), args.get(1)) else {
        bail!("usage: naval-vessel inspect <model.glb|model.gltf> [--write]");
    };
    let path = Path::new(path);
    let inspection = naval_vessel::files::inspect_file(path)?;
    print!(
        "{}",
        inspection.report(naval_vessel::setup::DEFAULT_TEXTURE_SIZE)
    );
    let setup = naval_vessel::files::setup_path(path);
    let model = path
        .file_name()
        .context("the model has no file name")?
        .to_string_lossy()
        .into_owned();
    let json = inspection.starter_json(&model);
    if args.iter().any(|a| a == "--write") {
        // Created only if no file is there, in one step: never over a setup another hand wrote meanwhile.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&setup)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(format!("{json}\n").as_bytes())
                    .with_context(|| format!("writing {}", setup.display()))?;
                println!(
                    "wrote {}: set her real length and draft, her funnels and lamps (ship space, m)",
                    setup.display()
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                println!("{} is there already: left as it is", setup.display());
            }
            Err(e) => return Err(e).with_context(|| format!("creating {}", setup.display())),
        }
    } else {
        println!("starter setup ({} with --write):\n{json}", setup.display());
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
