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

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b): (u32, u32) = (1, 0);
    for x in data {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

