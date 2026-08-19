use bitstream_io::{BitWriter, BitWrite, LittleEndian};

use std::ops::{Index};

pub fn to_zlib(data: &[u8]) -> Vec<u8> {
    let mut deflated = to_deflate_block_type1(data);

    deflated.splice(0..0, [0x78, 0x01]); // push front zlib signature
    deflated.extend_from_slice(&(adler32(data)).to_be_bytes()); // append adler32

    deflated
}

pub fn to_deflate_blocks(data: &[u8]) -> Vec<u8> {
    const MAX_STORED: usize = 65535; // 2^16 - 1

    let nblocks = data.len().div_ceil(MAX_STORED).max(1);
    let mut out = Vec::with_capacity(data.len() + 5*nblocks);

    for (i, c) in data.chunks(MAX_STORED).enumerate() {
        let len = c.len() as u16;

        out.push(u8::from(i+1 == nblocks));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }

    out
}

pub fn to_deflate_block_type1(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::endian(Vec::new(), LittleEndian);

    w.write_bit(true).unwrap();
    w.write::<2, u8>(0b01).unwrap();

    let stream = apply_lzss(data);

    encode_lzss_stream(&mut w, &FIXED_CODES, &stream);

    let eob = FIXED_CODES[LL][256];
    w.write_var::<u16>(eob.1.into(), eob.0).unwrap();

    w.byte_align().unwrap();

    w.into_writer()
}

/* LZSS PART */

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LzssElem {
    Literal(u8),
    Reference {length: u16, distance: u16},
}

pub fn apply_lzss(data: &[u8]) -> Vec<LzssElem> {
    use LzssElem::*;

    const BWIN_LEN: usize = 32768;
    const FWIN_LEN: usize = 258;

    let mut output: Vec<LzssElem> = Vec::new();

    let mut i = 0;
    while i < data.len() {
        let mut best_match: LzssElem = Literal(data[i]);

        for j in (i.saturating_sub(BWIN_LEN)..i).rev() {
            let mut len = 0;
            for (k, l) in (j..j+FWIN_LEN).zip(i..i+FWIN_LEN) {
                if l >= data.len() {break};
                if data[k] != data[l] {break};

                len += 1;
            }

            if len >= 3 {
                let cur_match = Reference {length: len as u16, distance: (i - j) as u16};
                match best_match {
                    Literal(_) => best_match = cur_match,
                    Reference {length: len_o, distance: _} => if len_o < len {best_match = cur_match},
                }
            }
        }

        output.push(best_match);

        match best_match {
            Literal(_) => i += 1,
            Reference {length: len, distance: _} => i += len as usize,
        }
    }

    output
}

/* HUFFMAN PART */

const MAX_BITS: usize = 15;

const fn rev(code: u16, len: u8) -> u16 {
    assert!(len as usize <= MAX_BITS && len != 0);
    code.reverse_bits() >> (16 - len)
}

// MAKES REVERSED CODES
// to work with single LittleEndian bit writer.
pub const fn huffman_from_lengths<const N: usize> (table: &mut [(u16, u8); N]) {
    let mut i = 0;
    let mut bl_count:  [usize; MAX_BITS+1] = [0; MAX_BITS+1];
    let mut next_code: [u16;   MAX_BITS+1] = [0; MAX_BITS+1];

    while i < N {
        assert!(table[i].1 <= MAX_BITS as u8);
        bl_count[table[i].1 as usize] = bl_count[table[i].1 as usize] + 1;
        i = i + 1;
    };

    let mut bits = 1;
    let mut code = 0;

    while bits <= MAX_BITS {
        code = (code + bl_count[bits-1]) << 1;
        next_code[bits] = code as u16;
        bits = bits + 1;
    };

    i = 0;

    while i < N {
        table[i].0 = rev(next_code[table[i].1 as usize], table[i].1);
        next_code[table[i].1 as usize] = next_code[table[i].1 as usize] + 1;
        i = i + 1;
    };

}

/* LZSS + HUFFMAN */
pub fn encode_lzss_stream(w: &mut BitWriter<Vec<u8>, LittleEndian>, table: &Htable, stream: &[LzssElem]) {
    use LzssElem::*;

    for e in stream {
        match e {
            Literal(c) => {
                let code = table[LL][*c as usize];
                w.write_var::<u16>(code.1.into(), code.0).unwrap();
            },
            Reference {length: len, distance: dist} => {
                let (c, extra_len, extra_value) = LL::huffman_code_for(*len);
                let code = table[LL][c as usize];

                w.write_var::<u16>(code.1.into(), code.0).unwrap();
                if extra_len > 0 {w.write_var::<u16>(extra_len as u32, extra_value).unwrap()};

                let (c, extra_len, extra_value) = Distance::huffman_code_for(*dist);
                let code = table[Distance][c as usize];

                w.write_var::<u16>(code.1.into(), code.0).unwrap();
                if extra_len > 0 {w.write_var::<u16>(extra_len as u32, extra_value).unwrap()};
            },
        }
    }
}

/* HUFFMAN TABLE */
type Code = u16;
type BitLen = u8;
type ExtraBits = u16;

/* ZST to index Htable */
pub struct LL;
pub struct Distance;

pub trait Symbol {
    type Table: ?Sized;
}

impl Symbol for LL       { type Table = [(Code, BitLen); 288]; }
impl Symbol for Distance { type Table = [(Code, BitLen); 32]; }

pub struct Htable {
    ll:       <LL       as Symbol>::Table,
    distance: <Distance as Symbol>::Table,
}

impl Index<LL> for Htable {
    type Output = <LL as Symbol>::Table;
    fn index(&self, _: LL) -> &Self::Output {
        &self.ll
    }
}

impl Index<Distance> for Htable {
    type Output = <Distance as Symbol>::Table;
    fn index(&self, _: Distance) -> &Self::Output {
        &self.distance
    }
}

impl LL {
    pub fn huffman_code_for(len: u16) -> (Code, BitLen, ExtraBits) {
        assert!(len >= 3 && len <= 258);

        if len <= 10 {(256 + len - 2, 0, 0)}
        else if len <= 18  {(265 + (len - 11) /2,  1, (len - 11)  % 2)}
        else if len <= 34  {(269 + (len - 19) /4,  2, (len - 19)  % 4)}
        else if len <= 66  {(273 + (len - 35) /8,  3, (len - 35)  % 8)}
        else if len <= 130 {(277 + (len - 67) /16, 4, (len - 67)  % 16)}
        else if len <= 257 {(281 + (len - 131)/32, 5, (len - 131) % 32)}
        else {(285, 0, 0)}
    }
}

impl Distance {
    pub fn huffman_code_for(dist: u16) -> (Code, BitLen, ExtraBits) {
        assert!(dist >= 1 && dist <= 32768);

        if dist <= 4 {return (dist - 1, 0, 0);}

        let d      = dist - 1;                        // turn dist to 10xx/11xx
        let n: u16 = (15 - d.leading_zeros()) as u16;
        let extra  = (n - 1) as u8;
        let code   = 2 * n + ((d >> extra) & 1);

        (code, extra, d & ((1 << extra) - 1))
    }
}

// CONTAINS REVERSED CODES
pub const FIXED_CODES: Htable = {
    let mut ll:         <LL       as Symbol>::Table = [(0, 0); 288];
    let mut distance:   <Distance as Symbol>::Table = [(0, 5); 32];
    let mut i: usize = 0;

    while i < 288 {
        ll[i].0 = i as u16;

        if      i <= 143 || i >= 280 {ll[i].1 = 8;}
        else if i >= 144 && i <= 255 {ll[i].1 = 9;}
        else if i >= 256 && i <= 279 {ll[i].1 = 7;}
        else    {panic!()}

        i += 1;
    };

    huffman_from_lengths(&mut ll);
    huffman_from_lengths(&mut distance);

    Htable {ll, distance}
};

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for x in data {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

