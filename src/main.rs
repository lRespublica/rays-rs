use std::env;

use std::path::PathBuf;

use std::io::BufWriter;
use std::io::Write;

use std::fs::File;

const PROG_NAME: &str = "rays-rs";

fn main() -> std::io::Result<()> {
    let mut args = env::args_os();

    let Some(filepath) = args.nth(1) else {
        eprintln!("usage: {PROG_NAME} <path>");
        std::process::exit(1);
    };

    let filepath: PathBuf = filepath.into();
    let file   = File::create(filepath)?;
    let mut w  = BufWriter::with_capacity(1024 * 64, file);

    let (nx, ny) = (1920, 1080);

    write!(&mut w, "P3\n{nx} {ny}\n255\n")?;

    for j in (0..ny).rev() {
        for i in 0..nx {
            let r: i32 = ((i as f32 / nx as f32) * 255.0).round() as i32;
            let g: i32 = ((j as f32 / ny as f32) * 255.0).round() as i32;
            let b = 51;

            write!(&mut w, "{r} {g} {b}\n")?;
        }
    }

    Ok(())
}
