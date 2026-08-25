use super::*;

// ------------------------------------------------------------------
// 0. Reference decoder
// ------------------------------------------------------------------

fn inflate_zlib_miniz(z: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec_zlib(z).expect("miniz_oxide: invalid zlib stream")
}

fn inflate_raw_miniz(d: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec(d).expect("miniz_oxide: ivalid deflate stream")
}

#[track_caller]
fn check_zlib(name: &str, data: &[u8]) {
    let z = to_zlib(data);
    assert_eq!(inflate_zlib_miniz(&z), data, "{name}: miniz_oxide/zlib");
}

#[track_caller]
fn check_raw(name: &str, data: &[u8], encoded: &[u8]) {
    assert_eq!(inflate_raw_miniz(encoded), data, "{name}: miniz_oxide/raw");
}

// ------------------------------------------------------------------
// 0.1 PRNG generator
// ------------------------------------------------------------------
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 32) as u8
    }
}

// ------------------------------------------------------------------
// 0.2 Input data corpus
// ------------------------------------------------------------------

fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |n: &str, d: Vec<u8>| v.push((n.to_string(), d));

    // degenerate sizes
    add("empty", vec![]);
    add("one byte", vec![0]);
    add("one byte 0xff", vec![0xff]);
    add("two bytes", vec![b'a', b'a']);
    add("three equal bytes", vec![b'a'; 3]); // minimal match length
    add("four equal bytes", vec![b'a'; 4]);

    // one symbol: LL-table degenerates to symbol + EOB
    add("5k zeros", vec![0; 5000]);
    add("258 zeros", vec![0; 258]); // max match length
    add("259 zeros", vec![0; 259]);
    add("260 zeros", vec![0; 260]);

    // no matches -> empty distance tree
    add("all 256 byte values once", (0..=255u8).collect());

    {
        let mut r = Rng::new(0xDEAD_BEEF);
        add("random 4k", (0..4000).map(|_| r.byte()).collect());
    }

    {
        let lorem = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit, \
                      sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ";
        add("lorem x30", lorem.iter().copied().cycle().take(lorem.len() * 30).collect());
    }

    add("fibonacci frequencies", fibonacci_input());

    v
}

// Worst case for huffman encoding
fn fibonacci_input() -> Vec<u8> {
    let mut d = Vec::new();
    let (mut a, mut b) = (1usize, 1usize);
    for sym in 0..15u8 {
        for _ in 0..a {
            d.push(sym);
            d.push(0x80 | sym);
        }
        let next = a + b;
        a = b;
        b = next;
    }
    d
}

// ==================================================================
// 1. Round-trip
// ==================================================================

#[test]
fn zlib_roundtrip_corpus() {
    for (name, data) in corpus() {
        check_zlib(&name, &data);
    }
}

#[test]
fn deflate_fixed_block_roundtrip_corpus() {
    for (name, data) in corpus() {
        let enc = to_deflate_block_type1(&data);
        check_raw(&format!("{name} (type 1)"), &data, &enc);
    }
}

#[test]
fn deflate_dynamic_block_roundtrip_corpus() {
    for (name, data) in corpus() {
        let enc = to_deflate_block_type2(&data);
        check_raw(&format!("{name} (type 2)"), &data, &enc);
    }
}

// ==================================================================
// 1.1 property-based
// ==================================================================

#[test]
fn zlib_roundtrip_random_bytes() {
    let mut r = Rng::new(1);
    for _ in 0..60 {
        let n = r.below(300) as usize;
        let data: Vec<u8> = (0..n).map(|_| r.byte()).collect();
        check_zlib("random bytes", &data);
    }
}

#[test]
fn deflate_all_block_types_roundtrip_random() {
    let mut r = Rng::new(4);
    for _ in 0..25 {
        let n = r.below(500) as usize;
        let data: Vec<u8> = (0..n).map(|_| r.byte() % 8).collect();
        check_raw("random/type1", &data, &to_deflate_block_type1(&data));
        check_raw("random/type2", &data, &to_deflate_block_type2(&data));
    }
}

// ==================================================================
// 2. zlib
// ==================================================================

#[test]
fn zlib_header_and_trailer() {
    for (name, data) in corpus() {
        let z = to_zlib(&data);
        assert!(z.len() >= 6, "{name}: too short stream");

        let cmf = z[0];
        let flg = z[1];
        assert_eq!(cmf & 0x0f, 8, "{name}: CM must be 8 (deflate)");
        assert!((cmf >> 4) <= 7, "{name}: CINFO should not be greater than 7");
        assert_eq!(flg & 0x20, 0, "{name}: FDICT must be 0");
        assert_eq!(
            (u16::from_be_bytes([cmf, flg])) % 31,
            0,
            "{name}: header's checksum (CMF*256+FLG) % 31 != 0"
        );

        let tail = &z[z.len() - 4..];
        assert_eq!(
            u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]),
            adler32(&data),
            "{name}: ADLER32 doesn't match"
        );

        // body between head and tail is valid raw deflate
        check_raw(&format!("{name} (zlib body)"), &data, &z[2..z.len() - 4]);
    }
}

// ==================================================================
// 3. adler32
// ==================================================================

#[test]
fn adler32_known_vectors() {
    assert_eq!(adler32(b""), 1);
    assert_eq!(adler32(b"a"), 0x0062_0062);
    assert_eq!(adler32(b"abc"), 0x024D_0127);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    assert_eq!(adler32(b"message digest"), 0x29750586);
    assert_eq!(adler32(&[0u8; 1000]), 0x03E8_0001);
}

// ==================================================================
// 4. LZSS
// ==================================================================

// Backward transformation
fn undo_lzss(stream: &[LzssElem]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for &e in stream {
        match e {
            LzssElem::Literal(c) => out.push(c),
            LzssElem::Reference { length, distance } => {
                let start = out
                    .len()
                    .checked_sub(distance as usize)
                    .expect("reference is out of bounds");
                for k in 0..length as usize {
                    let b = out[start + k];
                    out.push(b);
                }
            }
            LzssElem::EOB => break
        }
    }
    out
}

#[test]
fn lzss_roundtrip() {
    for (name, data) in corpus() {
        let stream = apply_lzss(&data);
        assert_eq!(undo_lzss(&stream), data, "{name}");
    }
}

#[test]
fn lzss_roundtrip_random() {
    let mut r = Rng::new(5);
    for _ in 0..100 {
        let n = r.below(800) as usize;
        let alphabet = 1 + r.below(6) as u8;
        let data: Vec<u8> = (0..n).map(|_| r.byte() % alphabet).collect();
        assert_eq!(undo_lzss(&apply_lzss(&data)), data);
    }
}

// ==================================================================
// 5. package_merge
// ==================================================================

// Kraft's sum should be equal to one exactly
// Unfull tree is declined by some decoders.
#[track_caller]
fn assert_kraft_complete(lengths: &[BitLen], max_bits: usize) {
    let total: u64 = lengths
        .iter()
        .filter(|&&l| l != 0)
        .map(|&l| 1u64 << (max_bits - l as usize))
        .sum();
    assert_eq!(
        total,
        1u64 << max_bits,
        "Kraft's sum != 1: {lengths:?}"
    );
}

#[test]
fn package_merge_kraft_is_complete() {
    let mut r = Rng::new(11);
    for _ in 0..200 {
        let mut freqs = [0u64; 32];
        for f in freqs.iter_mut() {
            *f = if r.below(4) == 0 { 0 } else { 1 + r.below(1000) };
        }
        for &max_bits in &[5usize, 7, 9, 15] {
            let lens = package_merge(&freqs, max_bits);
            assert_kraft_complete(&lens, max_bits);
        }
    }
}

#[test]
fn package_merge_respects_max_bits() {
    let mut r = Rng::new(12);
    for _ in 0..200 {
        let mut freqs = [0u64; 64];
        for (i, f) in freqs.iter_mut().enumerate() {
            *f = if r.below(3) == 0 { 0 } else { 1 << (i % 40) };
        }
        for &max_bits in &[7usize, 10, 15] {
            let lens = package_merge(&freqs, max_bits);
            for &l in lens.iter() {
                assert!(
                    l as usize <= max_bits,
                    "{l} excees {max_bits}"
                );
            }
            assert_kraft_complete(&lens, max_bits);
        }
    }
}

#[test]
fn package_merge_edge_cases() {
    let lens = package_merge(&[0u64; 32], 15);
    assert_eq!(lens[0], 1);
    assert_eq!(lens[1], 1);
    assert_eq!(lens[2..].iter().copied().max(), Some(0));
    assert_kraft_complete(&lens, 15);

    for c in [0usize, 1, 5, 31] {
        let mut freqs = [0u64; 32];
        freqs[c] = 42;
        let lens = package_merge(&freqs, 15);
        assert_eq!(lens[c], 1, "symbol {c} should achieve 1 bit code");
        assert_eq!(
            lens.iter().filter(|&&l| l != 0).count(),
            2,
            "only one virtual symbol should be added"
        );
        assert_kraft_complete(&lens, 15);
    }

    // exactly two symbols
    let mut freqs = [0u64; 32];
    freqs[3] = 1;
    freqs[9] = 1_000_000;
    let lens = package_merge(&freqs, 15);
    assert_eq!((lens[3], lens[9]), (1, 1));
    assert_kraft_complete(&lens, 15);

    // uniform distribution of powers of two -> all lengths should be equal
    let freqs = [7u64; 16];
    let lens = package_merge(&freqs, 15);
    assert!(lens.iter().all(|&l| l == 4), "achieved {lens:?}");

    // uniform distribution of non-even numbers
    let freqs = [1u64; 3];
    let lens = package_merge(&freqs, 15);
    let mut sorted = lens;
    sorted.sort_unstable();
    assert_eq!(sorted, [1, 2, 2]);

    // n == 1 << max_bits
    let freqs = [1u64; 16];
    let lens = package_merge(&freqs, 4);
    assert!(lens.iter().all(|&l| l == 4));
    assert_kraft_complete(&lens, 4);
}

#[test]
#[should_panic(expected = "no code with")]
fn package_merge_panics_when_alphabet_too_large() {
    // 17 cannot be encoded within maximum 4 bits
    let freqs = [1u64; 17];
    let _ = package_merge(&freqs, 4);
}

#[test]
fn package_merge_limits_fibonacci_worst_case() {
    const CAP: u64 = 1 << 40;
    let mut freqs = [0u64; 150];
    let (mut a, mut b) = (1u64, 1u64);
    for f in freqs.iter_mut() {
        *f = a;
        let next = (a + b).min(CAP);
        a = b.min(CAP);
        b = next;
    }
    let lens = package_merge(&freqs, 15);
    assert!(lens.iter().all(|&l| l <= 15), "achieved {lens:?}");
    assert!(
        lens.iter().any(|&l| l >= 14),
        "length limit probably is not exceeded: {lens:?}"
    );
    assert_kraft_complete(&lens, 15);
}

// ==================================================================
// 6. huffman_from_lengths / rev
// ==================================================================

#[test]
fn rev_is_involution() {
    assert_eq!(rev(0b1, 3), 0b100);
    assert_eq!(rev(0b110, 3), 0b011);
    assert_eq!(rev(0b0011_0000, 8), 0b0000_1100);
}

#[test]
fn huffman_from_lengths_known_example() {
    let table = huffman_from_lengths(&[2u8, 1, 3, 3]);
    let fwd: Vec<u16> = table.iter().map(|&(c, l)| rev(c, l)).collect();
    assert_eq!(fwd, vec![0b10, 0b0, 0b110, 0b111]);
}

#[test]
fn fixed_codes_match_rfc1951() {
    let ll = &FIXED_CODES.ll;
    let dist = &FIXED_CODES.distance;

    for i in 0..288usize {
        let want = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
        assert_eq!(ll[i].1, want, "Fixed LL-table, symbol {i}");
    }
    assert!(dist.iter().all(|&(_, l)| l == 5));

    let fwd = |i: usize| rev(ll[i].0, ll[i].1);
    assert_eq!(fwd(0), 0b0011_0000);
    assert_eq!(fwd(143), 0b1011_1111);
    assert_eq!(fwd(144), 0b1_1001_0000);
    assert_eq!(fwd(255), 0b1_1111_1111);
    assert_eq!(fwd(256), 0b000_0000);
    assert_eq!(fwd(279), 0b001_0111);
    assert_eq!(fwd(280), 0b1100_0000);
    assert_eq!(fwd(287), 0b1100_0111);

    for i in 0..32usize {
        assert_eq!(rev(dist[i].0, 5), i as u16, "fixed code for distance {i}");
    }
}

// ==================================================================
// 7. Lengths ans distance codes
// ==================================================================

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
    131, 163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
    13, 13,
];

#[test]
fn length_codes_match_rfc_exhaustively() {
    for len in 3u16..=258 {
        let (code, extra, value) = LL::huffman_code_for(len);
        assert!(
            (257..=285).contains(&code),
            "length {len}: code {code} is our 257..=285"
        );
        let i = code as usize - 257;
        assert_eq!(extra, LENGTH_EXTRA[i], "length {len}: amount of extra bits");
        assert!(
            extra == 0 || value < (1u16 << extra),
            "length {len}: value of {value} doesn't fit into {extra} extra bits"
        );
        assert_eq!(
            LENGTH_BASE[i] + value,
            len,
            "length {len}: base {} + {value} != {len}",
            LENGTH_BASE[i]
        );
    }
}

#[test]
fn length_codes_are_monotonic() {
    let mut prev = 256u16;
    for len in 3u16..=258 {
        let (code, _, _) = LL::huffman_code_for(len);
        assert!(code >= prev, "length {len}: code began to decrease");
        prev = code;
    }
    assert_eq!(LL::huffman_code_for(3).0, 257);
    assert_eq!(LL::huffman_code_for(258).0, 285);
}

#[test]
#[should_panic]
fn length_code_rejects_too_short() {
    let _ = LL::huffman_code_for(2);
}

#[test]
#[should_panic]
fn length_code_rejects_too_long() {
    let _ = LL::huffman_code_for(259);
}

#[test]
fn distance_codes_match_rfc_exhaustively() {
    for dist in 1u16..=32768 {
        let (code, extra, value) = Distance::huffman_code_for(dist);
        assert!(code <= 29, "distance {dist}: code {code} is out 0..=29");
        let i = code as usize;
        assert_eq!(extra, DIST_EXTRA[i], "distance {dist}: amount of extra bits");
        assert!(
            extra == 0 || value < (1u16 << extra),
            "distance {dist}: value {value} doesn't fit into {extra} bits"
        );
        assert_eq!(
            DIST_BASE[i] + value,
            dist,
            "distance {dist}: base {} + {value} != {dist}",
            DIST_BASE[i]
        );
    }
}

#[test]
fn distance_codes_are_monotonic() {
    let mut prev = 0u16;
    for dist in 1u16..=32768 {
        let (code, _, _) = Distance::huffman_code_for(dist);
        assert!(code >= prev, "distance {dist}: code began to decrease");
        prev = code;
    }
}

#[test]
#[should_panic]
fn distance_code_rejects_zero() {
    let _ = Distance::huffman_code_for(0);
}

// ==================================================================
// 8. Dynamic block tabel
// ==================================================================

const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn undo_cl(stream: &[CLElem]) -> Vec<BitLen> {
    let mut out: Vec<BitLen> = Vec::new();
    for &e in stream {
        match e {
            CLElem::CL(c) => out.push(c),
            CLElem::RPrevious(n) => {
                let prev = *out.last().expect("RPrevious as first element");
                for _ in 0..(n as usize + 3) {
                    out.push(prev);
                }
            }
            CLElem::RZeroS(n) => {
                for _ in 0..(n as usize + 3) {
                    out.push(0);
                }
            }
            CLElem::RZeroL(n) => {
                for _ in 0..(n as usize + 11) {
                    out.push(0);
                }
            }
        }
    }
    out
}

fn tables_for(data: &[u8]) -> Htable {
    Htable::from_stream(&apply_lzss(data))
}

#[test]
fn cl_stream_roundtrip() {
    for (name, data) in corpus() {
        let t = tables_for(&data);
        let (stream, hlit_m257, hdist_m1) = t.encode();
        let hlit = hlit_m257 as usize + 257;
        let hdist = hdist_m1 as usize + 1;

        let want: Vec<BitLen> = t.ll[..hlit]
            .iter()
            .chain(&t.distance[..hdist])
            .map(|&(_, l)| l)
            .collect();

        assert_eq!(undo_cl(&stream), want, "{name}: CL stream cannot be restored");
    }
}

#[test]
fn hlit_hdist_hclen_in_range() {
    for (name, data) in corpus() {
        let t = tables_for(&data);
        let (stream, hlit_m257, hdist_m1) = t.encode();

        assert!(
            hlit_m257 <= 29,
            "{name}: HLIT-257 = {hlit_m257} > 29"
        );
        assert!(hdist_m1 <= 29, "{name}: HDIST-1 = {hdist_m1} > 29");

        assert_eq!(t.ll[286].1, 0, "{name}: symbol 286 achieved code");
        assert_eq!(t.ll[287].1, 0, "{name}: symbol 287 achieved code");
        assert_ne!(t.ll[256].1, 0, "{name}: EOB must have it's own code");

        let cl_table = CL::generate_huffman_code(&stream);
        let hclen = CL_ORDER
            .iter()
            .rposition(|&i| cl_table[i].1 != 0)
            .map_or(4, |p| (p + 1).max(4));
        assert!(hclen >= 4 && hclen <= 19, "{name}: HCLEN+4 = {hclen}");
        for &i in CL_ORDER.iter().skip(hclen) {
            assert_eq!(
                cl_table[i].1, 0,
                "{name}: CL-symbol {i} have code, but it's not encoded"
            );
        }
    }
}
