use std::io;
use std::io::Write;

use bytemuck;

use rayon::prelude::*;

use crate::color::RGB;
use crate::image::Image;

use crate::algorithms::zlib::to_zlib;

pub fn write<W: Write>(img: &Image<RGB>, w: &mut W) -> io::Result<()> {
    let ihdr: [u8; 13] = {
        let (w, h) = (img.width().to_be_bytes(), img.height().to_be_bytes());
        [
            w[0], w[1], w[2], w[3], h[0], h[1], h[2], h[3], 8, 2, 0, 0, 0,
        ]
    };

    let stride = img.width() as usize * 3 + 1;
    let mut idat_raw = vec![0u8; img.height() as usize * stride];

    idat_raw
        .par_chunks_mut(stride)
        .zip_eq(img.raw().par_chunks(img.width() as usize))
        .for_each(|(dst, row)| {
            dst[0] = 0;
            dst[1..].copy_from_slice(bytemuck::cast_slice(row));
        });

    let idat = to_zlib(&idat_raw);

    w.write_all(b"\x89PNG\r\n\x1a\n")?;
    write_chunk(w, b"IHDR", &ihdr)?;
    write_chunk(w, b"IDAT", &idat)?;
    write_chunk(w, b"IEND", b"")?;

    Ok(())
}

fn write_chunk<W: Write>(w: &mut W, tag: &[u8; 4], data: &[u8]) -> io::Result<()> {
    let crc = crc32(tag);
    let crc = crc32_update(crc, data);

    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(tag)?;
    w.write_all(data)?;
    w.write_all(&crc.to_be_bytes())?;

    Ok(())
}

/* INTERNAL */

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 == 1 {
                0xEDB88320 ^ (c >> 1)
            } else {
                c >> 1
            };
            j = j + 1;
        }
        table[i] = c;
        i = i + 1;
    }

    table
};

pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut c: u32 = !crc;
    for d in data {
        c = CRC_TABLE[usize::from((c as u8) ^ d)] ^ (c >> 8);
    }

    c ^ 0xFFFFFFFF
}

pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for x in data {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
