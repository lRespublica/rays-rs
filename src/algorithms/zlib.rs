use bitstream_io::{BitWrite, BitWriter, LittleEndian};

use std::{io, ops::Index};

use itertools::Itertools;

pub fn to_zlib(data: &[u8]) -> Vec<u8> {
    let mut deflated = to_deflate_block_type1(data);

    deflated.splice(0..0, [0x78, 0x01]); // push front zlib signature
    deflated.extend_from_slice(&(adler32(data)).to_be_bytes()); // append adler32

    deflated
}

pub fn to_deflate_blocks(data: &[u8]) -> Vec<u8> {
    const MAX_STORED: usize = 65535; // 2^16 - 1

    let nblocks = data.len().div_ceil(MAX_STORED).max(1);
    let mut out = Vec::with_capacity(data.len() + 5 * nblocks);

    for (i, c) in data.chunks(MAX_STORED).enumerate() {
        let len = c.len() as u16;

        out.push(u8::from(i + 1 == nblocks));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }

    out
}

pub fn to_deflate_block_type1(data: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(Vec::<u8>::new());
    let stream = apply_lzss(data);

    encoder.fixed_block(true, &stream).unwrap();
    encoder.finish().unwrap()
}

/* LZSS PART */

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LzssElem {
    Literal(u8),
    Reference { length: u16, distance: u16 },
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
            for (k, l) in (j..j + FWIN_LEN).zip(i..i + FWIN_LEN) {
                if l >= data.len() {
                    break;
                };
                if data[k] != data[l] {
                    break;
                };

                len += 1;
            }

            if len >= 3 {
                let cur_match = Reference {
                    length: len as u16,
                    distance: (i - j) as u16,
                };
                match best_match {
                    Literal(_) => best_match = cur_match,
                    Reference {
                        length: len_o,
                        distance: _,
                    } => {
                        if len_o < len {
                            best_match = cur_match
                        }
                    }
                }
            }
        }

        output.push(best_match);

        match best_match {
            Literal(_) => i += 1,
            Reference {
                length: len,
                distance: _,
            } => i += len as usize,
        }
    }

    output
}

/* HUFFMAN PART */

const MAX_BITS: usize = 15;

type Code = u16;
type BitLen = u8;
type ExtraBits = u16;

type Frequency = u64;

pub fn package_merge(table: &[(Code, Frequency)], max_bits: usize) -> Vec<(Code, BitLen)> {
    type AIndex = u32; // Arena Index
    const NIL: AIndex = AIndex::MAX;

    // (head, tail, freq)
    type Element = (AIndex, AIndex, Frequency);

    assert!(!table.is_empty());
    assert!(table[0].0 == 0);
    assert!(table.windows(2).all(|w| w[0].0 + 1 == w[1].0));

    let mut leaves: Vec<(Code, Frequency)> =
        table.iter().copied().filter(|&(_, f)| f != 0).collect();

    leaves.sort_unstable_by_key(|&(c, f)| (f, c));

    let n = leaves.len();
    assert!(
        n >= 2,
        "package_merge requires at least 2 symbols with nonzero frequency"
    );
    assert!(
        n <= 1 << max_bits,
        "no code with {max_bits} exists for {n} symbols"
    );

    // (code, next)
    let mut arena: Vec<(Code, AIndex)> = Vec::with_capacity(n * max_bits);
    let mut stored: Vec<Element> = Vec::with_capacity(n - 1);
    let mut row: Vec<Element> = Vec::with_capacity(2 * n - 1);

    for i in (1..=max_bits).rev() {
        let leaf_iter = leaves.iter().map(|&(c, f)| {
            let idx = arena.len() as AIndex;
            arena.push((c, NIL));
            (idx, idx, f)
        });

        row.extend(leaf_iter.merge_by(stored.drain(..), |a, b| a.2 <= b.2));

        if i == 1 {
            break;
        }

        for pair in row.chunks_exact(2) {
            let (h1, t1, f1) = pair[0];
            let (h2, t2, f2) = pair[1];
            arena[t1 as usize].1 = h2;
            stored.push((h1, t2, f1 + f2));
        }

        row.clear();
    }

    row.truncate(2 * n - 2);

    let mut ret: Vec<(Code, BitLen)> = table.iter().map(|&(c, _)| (c, 0)).collect();

    for &(head, _, _) in &row {
        let mut node = head;
        while node != NIL {
            let (c, next) = arena[node as usize];
            ret[c as usize].1 += 1;
            node = next;
        }
    }

    ret
}

const fn rev(code: u16, len: u8) -> u16 {
    assert!(len as usize <= MAX_BITS && len != 0);
    code.reverse_bits() >> (16 - len)
}

// MAKES REVERSED CODES
// to work with single LittleEndian bit writer.
pub const fn huffman_from_lengths<const N: usize>(table: &mut [(u16, u8); N]) {
    let mut i = 0;
    let mut bl_count: [usize; MAX_BITS + 1] = [0; MAX_BITS + 1];
    let mut next_code: [u16; MAX_BITS + 1] = [0; MAX_BITS + 1];

    while i < N {
        assert!(table[i].1 <= MAX_BITS as u8);
        bl_count[table[i].1 as usize] += 1;
        i = i + 1;
    }

    let mut bits = 1;
    let mut code = 0;

    bl_count[0] = 0;

    while bits <= MAX_BITS {
        code = (code + bl_count[bits - 1]) << 1;
        next_code[bits] = code as u16;
        bits += 1;
    }

    i = 0;

    while i < N {
        let len = table[i].1 as usize;
        if len != 0 {
            table[i].0 = rev(next_code[len], table[i].1);
            next_code[len] += 1;
        } else {
            table[i].0 = 0;
        }
        i = i + 1;
    }
}

/* HUFFMAN TABLE */

/* ZST to index Htable */
pub struct LL;
pub struct Distance;

pub trait Symbol {
    type Table: ?Sized;
}

impl Symbol for LL {
    type Table = [(Code, BitLen); 288];
}
impl Symbol for Distance {
    type Table = [(Code, BitLen); 32];
}

pub struct Htable {
    ll: <LL as Symbol>::Table,
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

        if len <= 10 {
            (256 + len - 2, 0, 0)
        } else if len <= 18 {
            (265 + (len - 11) / 2, 1, (len - 11) % 2)
        } else if len <= 34 {
            (269 + (len - 19) / 4, 2, (len - 19) % 4)
        } else if len <= 66 {
            (273 + (len - 35) / 8, 3, (len - 35) % 8)
        } else if len <= 130 {
            (277 + (len - 67) / 16, 4, (len - 67) % 16)
        } else if len <= 257 {
            (281 + (len - 131) / 32, 5, (len - 131) % 32)
        } else {
            (285, 0, 0)
        }
    }
}

impl Distance {
    pub fn huffman_code_for(dist: u16) -> (Code, BitLen, ExtraBits) {
        assert!(dist >= 1 && dist <= 32768);

        if dist <= 4 {
            return (dist - 1, 0, 0);
        }

        let d = dist - 1; // turn dist to 10xx/11xx
        let n: u16 = (15 - d.leading_zeros()) as u16;
        let extra = (n - 1) as u8;
        let code = 2 * n + ((d >> extra) & 1);

        (code, extra, d & ((1 << extra) - 1))
    }
}

// CONTAINS REVERSED CODES
pub static FIXED_CODES: Htable = {
    let mut ll: <LL as Symbol>::Table = [(0, 0); 288];
    let mut distance: <Distance as Symbol>::Table = [(0, 5); 32];
    let mut i: usize = 0;

    while i < 288 {
        ll[i].0 = i as u16;

        if i <= 143 || i >= 280 {
            ll[i].1 = 8;
        } else if i >= 144 && i <= 255 {
            ll[i].1 = 9;
        } else if i >= 256 && i <= 279 {
            ll[i].1 = 7;
        } else {
            panic!()
        }

        i += 1;
    }

    huffman_from_lengths(&mut ll);
    huffman_from_lengths(&mut distance);

    Htable { ll, distance }
};

/* DEFLATE ENCODER */
pub struct Encoder<W: io::Write> {
    w: BitWriter<W, LittleEndian>,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BlockType {
    Stored = 0b00,
    Fixed = 0b01,
    Dynamic = 0b10,
}

struct HuffmanBlock<'w, 't, W: io::Write> {
    w: &'w mut BitWriter<W, LittleEndian>,
    table: &'t Htable,
}

impl<W: io::Write> Encoder<W> {
    pub fn new(w: W) -> Self {
        Encoder {
            w: BitWriter::endian(w, LittleEndian),
        }
    }

    /* PUBLIC INTERFACE */
    pub fn fixed_block(&mut self, is_final: bool, stream: &[LzssElem]) -> io::Result<()> {
        self.block_header(is_final, BlockType::Fixed)?;
        HuffmanBlock::new(&mut self.w, &FIXED_CODES).body(stream)
    }

    pub fn finish(mut self) -> io::Result<W> {
        self.w.byte_align()?;
        Ok(self.w.into_writer())
    }

    /* HELPER */
    fn block_header(&mut self, is_final: bool, btype: BlockType) -> io::Result<()> {
        self.w.write_bit(is_final)?;
        self.w.write::<2, u8>(btype as u8)
    }
}

impl<'w, 't, W: io::Write> HuffmanBlock<'w, 't, W> {
    fn new(w: &'w mut BitWriter<W, LittleEndian>, table: &'t Htable) -> Self {
        HuffmanBlock { w, table }
    }

    /* INTERFACE */

    /* consumes HuffmanBlock, so it's impossible to call .body() twice */
    fn body(mut self, stream: &[LzssElem]) -> io::Result<()> {
        use LzssElem::*;

        for &e in stream {
            match e {
                Literal(c) => self.literal(c)?,
                Reference {
                    length: len,
                    distance: dist,
                } => self.reference(len, dist)?,
            }
        }

        self.end_of_block()
    }

    /* FIRST LEVEL HELPERS */
    fn end_of_block(&mut self) -> io::Result<()> {
        self.code(self.table[LL][256])
    }

    fn literal(&mut self, lit: u8) -> io::Result<()> {
        self.code(self.table[LL][lit as usize])
    }

    fn reference(&mut self, len: u16, dist: u16) -> io::Result<()> {
        let (sym, bit_len, value) = LL::huffman_code_for(len);
        self.code(self.table[LL][sym as usize])?;
        self.extra(bit_len, value)?;

        let (sym, bit_len, value) = Distance::huffman_code_for(dist);
        self.code(self.table[Distance][sym as usize])?;
        self.extra(bit_len, value)
    }

    /* LOWEST LEVEL HELPERS */

    /* codes are stored pre-reversed: LSB-first write yields DEFLATE bit order */
    fn code(&mut self, (code, bit_len): (Code, BitLen)) -> io::Result<()> {
        self.w.write_var(bit_len as u32, code)
    }

    fn extra(&mut self, len: BitLen, value: ExtraBits) -> io::Result<()> {
        if len > 0 {
            self.w.write_var(len.into(), value)?
        };

        Ok(())
    }
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for x in data {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
