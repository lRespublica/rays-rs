pub fn to_zlib(data: &[u8]) -> Vec<u8> {
    let mut deflated = to_deflate_blocks(data);

    deflated.splice(0..0, [0x78, 0x01]); // push front zlib signature
    deflated.extend_from_slice(&(adler32(data)).to_be_bytes()); // append adler32

    deflated
}

fn to_deflate_blocks(data: &[u8]) -> Vec<u8> {
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

/* INTERNAL */

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

// CONTAINS REVERSED CODES
pub const FIXED_CODES: [(u16, u8); 288] = {
    let mut table: [(u16, u8); 288] = [(0, 0); 288];
    let mut i: usize = 0;

    while i < 288 {
        table[i].0 = i as u16;

        if      i <= 143 || i >= 280 {table[i].1 = 8;}
        else if i >= 144 && i <= 255 {table[i].1 = 9;}
        else if i >= 256 && i <= 279 {table[i].1 = 7;}
        else    {panic!()}

        i = i+1;
    };

    huffman_from_lengths(&mut table);

    table
};

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for x in data {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

