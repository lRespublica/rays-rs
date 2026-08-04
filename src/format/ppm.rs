use std::io::{self, Write};

use bytemuck;

use crate::color::RGB;
use crate::image::Image;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Encoding {
    Ascii,  // P3
    Binary, // P6
}

pub fn write<W: Write> (img: &Image<RGB>, encoding: Encoding, w: &mut W) -> io::Result<()> {
    match encoding {
        Encoding::Ascii  => write_ascii(img, w),
        Encoding::Binary => write_binary(img, w),
    }
}

fn write_binary<W: Write> (img: &Image<RGB>, w: &mut W) -> io::Result<()> {
    write!(w, "P6\n{} {}\n255\n", img.width(), img.height())?;
    w.write_all(bytemuck::cast_slice(img.raw()))
}

fn write_ascii<W: Write> (img: &Image<RGB>, w: &mut W) -> io::Result<()> {
    // PPM limits the line length to 70 symbols
    const PER_LINE: usize = 5; // length("255 255 255 ") * 4 == 60

    let write_rgb = |w: &mut dyn Write, c: &RGB| {write!(w, "{} {} {} ", c.r(), c.g(), c.b())};

    write!(w, "P3\n{} {}\n255\n", img.width(), img.height())?;

    for chunk in img.raw().chunks(PER_LINE) {
        for c in chunk {write_rgb(w, c)?};
        write!(w, "\n")?;
    }

    Ok(())
}
