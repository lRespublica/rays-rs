use bitstream_io::{BitWrite, BitWriter, LittleEndian};

use std::{io, ops::Index};

use rayon::prelude::*;

use itertools::Itertools;

static CHUNK: usize = 512 * 1024; // 512 KiB

pub fn to_zlib(data: &[u8]) -> Vec<u8> {
    let n_chunks = data.len().div_ceil(CHUNK).max(1);

    let parts: Vec<Vec<u8>> = (0..n_chunks)
        .into_par_iter()
        .map(|i| {
            let start = i * CHUNK;
            let end = (start + CHUNK).min(data.len());
            compress_chunk(&data[start..end], i + 1 == n_chunks)
        })
        .collect();

    let mut out: Vec<u8> = Vec::with_capacity(2 + parts.iter().map(Vec::len).sum::<usize>() + 4);

    out.extend_from_slice(&[0x78, 0x01]);
    for p in &parts {
        out.extend_from_slice(&p);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());

    out
}

fn compress_chunk(data: &[u8], is_last: bool) -> Vec<u8> {
    let stream = apply_lzss(data);

    let mut encoder = Encoder::new(Vec::<u8>::new());

    encoder.dynamic_block(is_last, &stream).unwrap();
    if !is_last {
        encoder.sync_flush().unwrap();
    }

    encoder.finish().unwrap()
}

/* LZSS PART */

#[derive(Debug, Copy, Clone, PartialEq)]
enum LzssElem {
    Literal(u8),
    Reference { length: u16, distance: u16 },
    EOB,
}

fn apply_lzss(data: &[u8]) -> Vec<LzssElem> {
    use LzssElem::*;

    const BWIN_LEN: usize = 8192;
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
                    EOB => {
                        unreachable!("EOB could not be matched")
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
            EOB => {
                unreachable!("EOB could not be matched")
            }
        }
    }

    output.push(EOB);

    output
}

/* HUFFMAN PART */

const MAX_BITS: usize = 15;

type Code = u16;
type BitLen = u8;
type ExtraBits = u16;

type Frequency = u64;

fn package_merge<const N: usize>(freqs: &[Frequency; N], max_bits: usize) -> [BitLen; N] {
    const { assert!(N >= 2) };

    type AIndex = u32; // Arena Index
    const NIL: AIndex = AIndex::MAX;

    // (head, tail, freq)
    type Element = (AIndex, AIndex, Frequency);

    let mut leaves: Vec<(Code, Frequency)> = freqs
        .iter()
        .copied()
        .enumerate()
        .map(|(c, f)| (c as Code, f))
        .filter(|&(_, f)| f != 0)
        .collect();

    // extend leaves with virtual symbols for one element and empty table cases
    match leaves.as_slice() {
        [] => {
            leaves.extend([(0, 1), (1, 1)]);
        }
        &[(c, _)] => leaves.push((Code::from(c == 0), 1)),
        _ => {}
    }

    leaves.sort_unstable_by_key(|&(c, f)| (f, c));

    let n = leaves.len();
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

    let mut ret: [BitLen; N] = [0; N];

    for &(head, _, _) in &row {
        let mut node = head;
        while node != NIL {
            let (c, next) = arena[node as usize];
            ret[c as usize] += 1;
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
const fn huffman_from_lengths<const N: usize>(lengths: &[BitLen; N]) -> [(Code, BitLen); N] {
    let mut i = 0;
    let mut bl_count: [usize; MAX_BITS + 1] = [0; MAX_BITS + 1];
    let mut next_code: [u16; MAX_BITS + 1] = [0; MAX_BITS + 1];

    let mut ret: [(Code, BitLen); N] = [(0, 0); N];

    while i < N {
        assert!(lengths[i] <= MAX_BITS as u8);
        bl_count[lengths[i] as usize] += 1;

        i += 1;
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
        let len = lengths[i] as usize;
        if len != 0 {
            ret[i].0 = rev(next_code[len], len as u8);
            ret[i].1 = len as u8;
            next_code[len] += 1;
        } else {
            ret[i].0 = 0;
            ret[i].1 = 0;
        }
        i += 1;
    }

    ret
}

/* HUFFMAN TABLE */

/* ZST to index Htable */
struct LL; // Literal & Lengths
struct Distance;
struct CL; // Code Lengths

trait Symbol {
    const N: usize;
    type Lengths;
    type Table;
}

impl Symbol for LL {
    const N: usize = 288;
    type Lengths = [BitLen; Self::N];
    type Table = [(Code, BitLen); Self::N];
}

impl Symbol for Distance {
    const N: usize = 32;
    type Lengths = [BitLen; Self::N];
    type Table = [(Code, BitLen); Self::N];
}

impl Symbol for CL {
    const N: usize = 19;
    type Lengths = [BitLen; Self::N];
    type Table = [(Code, BitLen); Self::N];
}

/* Code Lengths type */
#[derive(Debug, Copy, Clone, PartialEq)]
enum CLElem {
    CL(u8),        // Represent code lengths of 0 - 15
    RPrevious(u8), // Repeat the previous code 3 - 6 times. (2 bits of length)
    RZeroS(u8),    // Repeat Zero code Small version. 3 - 10 times. (3 bits of length)
    RZeroL(u8),    // Repeat Zero code Large version. 11 - 138 times. (7 bits of length)
}

#[derive(Debug, Clone)]
struct Htable {
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
    fn huffman_code_for(len: u16) -> (Code, BitLen, ExtraBits) {
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
    fn huffman_code_for(dist: u16) -> (Code, BitLen, ExtraBits) {
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

impl Htable {
    fn from_stream(stream: &[LzssElem]) -> Self {
        // last element of stream must be EOB marker
        assert!(
            stream.iter().rposition(|e| *e == LzssElem::EOB) == Some(stream.len() - 1),
            "from_stream: EOB must occur exactly once, at the end"
        );

        let mut ll: [Frequency; LL::N] = [0; LL::N];
        let mut distance: [Frequency; Distance::N] = [0; Distance::N];

        for &e in stream {
            match e {
                LzssElem::Literal(c) => ll[c as usize] += 1,
                LzssElem::Reference {
                    length: len,
                    distance: dist,
                } => {
                    let (c, _, _) = LL::huffman_code_for(len);
                    ll[c as usize] += 1;

                    let (c, _, _) = Distance::huffman_code_for(dist);
                    distance[c as usize] += 1;
                }
                LzssElem::EOB => ll[256] += 1,
            }
        }

        let ll = huffman_from_lengths(&package_merge(&ll, MAX_BITS));
        let distance = huffman_from_lengths(&package_merge(&distance, MAX_BITS));
        Htable { ll, distance }
    }

    // (elements, HLIT, HDIST)
    fn encode(&self) -> (Vec<CLElem>, u8, u8) {
        const CL_ZERO_MAX: usize = 138;
        const CL_REPEAT_MAX: usize = 6;

        let hlit = self
            .ll
            .iter()
            .rposition(|&(_, l)| l != 0)
            .map_or(257, |x| x + 1);
        let hdist = self
            .distance
            .iter()
            .rposition(|&(_, l)| l != 0)
            .map_or(1, |x| x + 1);

        assert!(hlit >= 257);
        assert!(hdist >= 1);

        let lens: Vec<BitLen> = self.ll[..hlit]
            .iter()
            .chain(&self.distance[..hdist])
            .map(|&(_, l)| l)
            .collect();

        let mut out = Vec::with_capacity(lens.len());
        let mut prev: Option<BitLen> = None;
        let mut i = 0;

        while i < lens.len() {
            let cur = lens[i];
            let mut run = 1;
            while i + run < lens.len() && lens[i + run] == cur && run < CL_ZERO_MAX {
                run += 1;
            }

            if cur == 0 {
                if run < 3 {
                    out.extend(std::iter::repeat_n(CLElem::CL(0), run));
                } else if run <= 10 {
                    out.push(CLElem::RZeroS((run - 3) as u8));
                } else {
                    out.push(CLElem::RZeroL((run - 11) as u8));
                }
            } else {
                let mut rest = run;
                if prev != Some(cur) {
                    out.push(CLElem::CL(cur));
                    rest -= 1;
                }

                while rest >= 3 {
                    let take = rest.min(CL_REPEAT_MAX);
                    out.push(CLElem::RPrevious((take - 3) as u8));
                    rest -= take;
                }

                out.extend(std::iter::repeat_n(CLElem::CL(cur), rest));
            }

            prev = Some(cur);
            i += run;
        }

        (out, (hlit - 257) as u8, (hdist - 1) as u8)
    }
}

impl CL {
    fn generate_huffman_code(stream: &[CLElem]) -> <Self as Symbol>::Table {
        let mut freqs: [Frequency; Self::N] = [0; Self::N];

        for &e in stream {
            match e {
                CLElem::CL(c) => freqs[c as usize] += 1,
                CLElem::RPrevious(_) => freqs[16] += 1,
                CLElem::RZeroS(_) => freqs[17] += 1,
                CLElem::RZeroL(_) => freqs[18] += 1,
            }
        }

        huffman_from_lengths(&package_merge(&freqs, 7))
    }
}

// CONTAINS REVERSED CODES
static FIXED_CODES: Htable = {
    let mut ll: <LL as Symbol>::Lengths = [0; 288];
    let distance: <Distance as Symbol>::Lengths = [5; 32];
    let mut i: usize = 0;

    while i < 288 {
        if i <= 143 {
            ll[i] = 8;
        } else if i >= 144 && i <= 255 {
            ll[i] = 9;
        } else if i >= 256 && i <= 279 {
            ll[i] = 7;
        } else if i >= 280 && i < 288 {
            ll[i] = 8;
        } else {
            unreachable!()
        }

        i += 1;
    }

    let ll = huffman_from_lengths(&ll);
    let distance = huffman_from_lengths(&distance);

    Htable { ll, distance }
};

/* DEFLATE ENCODER */
struct Encoder<W: io::Write> {
    w: BitWriter<W, LittleEndian>,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum BlockType {
    Stored = 0b00,
    Fixed = 0b01,
    Dynamic = 0b10,
}

struct HuffmanBlock<'w, 't, W: io::Write> {
    w: &'w mut BitWriter<W, LittleEndian>,
    table: &'t Htable,
}

impl<W: io::Write> Encoder<W> {
    fn new(w: W) -> Self {
        Encoder {
            w: BitWriter::endian(w, LittleEndian),
        }
    }

    /* PUBLIC INTERFACE */
    #[allow(dead_code)]
    fn fixed_block(&mut self, is_final: bool, stream: &[LzssElem]) -> io::Result<()> {
        self.block_header(is_final, BlockType::Fixed)?;
        HuffmanBlock::new(&mut self.w, &FIXED_CODES).body(stream)
    }

    fn dynamic_block(&mut self, is_final: bool, stream: &[LzssElem]) -> io::Result<()> {
        self.block_header(is_final, BlockType::Dynamic)?;

        let table = Htable::from_stream(stream);
        let mut writer = HuffmanBlock::new(&mut self.w, &table);

        writer.table()?;
        writer.body(stream)
    }

    fn sync_flush(&mut self) -> io::Result<()> {
        self.block_header(false, BlockType::Stored)?;
        self.w.byte_align()?;
        self.w.write::<16, u16>(0)?;
        self.w.write::<16, u16>(0xFFFF)
    }

    fn finish(mut self) -> io::Result<W> {
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
                EOB => self.end_of_block()?,
            }
        }

        Ok(())
    }

    fn table(&mut self) -> io::Result<()> {
        const CL_ORDER: [usize; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];

        let (cl_stream, hlit, hdist) = self.table.encode();
        let cl_table = CL::generate_huffman_code(&cl_stream);

        let hclen: u8 = CL_ORDER
            .iter()
            .rposition(|&i| cl_table[i].1 != 0)
            .map_or(4, |p| (p + 1).max(4) as u8);
        let hclen: u8 = hclen - 4;

        self.w.write_var(5, hlit)?;
        self.w.write_var(5, hdist)?;
        self.w.write_var(4, hclen)?;

        for &i in CL_ORDER.iter().take((hclen + 4) as usize) {
            self.w.write_var(3, cl_table[i].1)?;
        }

        for e in cl_stream {
            match e {
                CLElem::CL(c) => {
                    let (val, len) = cl_table[c as usize];
                    self.w.write_var(len as u32, val)?;
                }
                CLElem::RPrevious(c) => {
                    let (val, len) = cl_table[16];
                    self.w.write_var(len as u32, val)?;
                    self.w.write_var(2, c)?;
                }
                CLElem::RZeroS(c) => {
                    let (val, len) = cl_table[17];
                    self.w.write_var(len as u32, val)?;
                    self.w.write_var(3, c)?;
                }
                CLElem::RZeroL(c) => {
                    let (val, len) = cl_table[18];
                    self.w.write_var(len as u32, val)?;
                    self.w.write_var(7, c)?;
                }
            }
        }

        Ok(())
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

#[cfg(test)]
mod tests;
