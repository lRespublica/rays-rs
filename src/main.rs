use std::env;

use std::path::PathBuf;

use std::io::BufWriter;

use std::fs::File;

use rays_rs::color::Color;
use rays_rs::image::Image;

use rays_rs::format::png;

const PROG_NAME: &str = "rays-rs";

fn main() -> std::io::Result<()> {
    let mut args = env::args_os();

    let Some(filepath) = args.nth(1) else {
        eprintln!("usage: {PROG_NAME} <path>");
        std::process::exit(1);
    };

    let filepath: PathBuf = filepath.into();
    let file = File::create(filepath)?;
    let mut w = BufWriter::with_capacity(1024 * 1024, file);

    let (nx, ny) = (1920, 1080);

    let img = Image::<Color>::from_fn(nx, ny, |x, y| {
        Color::new(x as f32 / nx as f32, (ny - y) as f32 / ny as f32, 0.25)
    });

    png::write(&img.to_rgb(), &mut w)?;

    Ok(())
}
