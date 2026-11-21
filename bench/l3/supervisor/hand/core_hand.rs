struct Decoder {
    remaining: u8,
    cp: u32,
    lower: u8,
    upper: u8,
}

impl Decoder {
    fn new() -> Self {
        Self { remaining: 0, cp: 0, lower: 0x80, upper: 0xbf }
    }
}

pub(crate) struct Tail {
    decoders: [Decoder; 2],
    bytes: Vec<u8>,
    units: usize,
    limit: usize,
}

impl Tail {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            decoders: [Decoder::new(), Decoder::new()],
            bytes: Vec::with_capacity(6 * limit + 4),
            units: 0,
            limit,
        }
    }

    fn trim(&mut self) {
        let mut start = 0;
        let len = self.bytes.len();
        while self.units > self.limit || len - start > 3 * self.limit {
            let width = match self.bytes[start] {
                0..=0x7f => 1,
                0x80..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            start += width;
            self.units -= if width == 4 { 2 } else { 1 };
        }
        if start != 0 {
            self.bytes.copy_within(start.., 0);
            self.bytes.truncate(len - start);
        }
    }

    fn append(&mut self, ch: char) {
        let mut encoded = [0; 4];
        self.bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        self.units += ch.len_utf16();
        if self.units > 2 * self.limit || self.bytes.len() > 6 * self.limit {
            self.trim();
        }
    }

    pub(crate) fn push(&mut self, stream: usize, bytes: &[u8]) {
        for &byte in bytes {
            let decoder = &mut self.decoders[stream];
            if decoder.remaining != 0 {
                if (decoder.lower..=decoder.upper).contains(&byte) {
                    decoder.remaining -= 1;
                    decoder.cp = (decoder.cp << 6) | u32::from(byte & 0x3f);
                    decoder.lower = 0x80;
                    decoder.upper = 0xbf;
                    if decoder.remaining == 0 {
                        let ch = char::from_u32(decoder.cp).unwrap();
                        self.append(ch);
                    }
                    continue;
                }
                *decoder = Decoder::new();
                self.append(char::REPLACEMENT_CHARACTER);
            }

            let (remaining, mask, lower, upper) = match byte {
                0x00..=0x7f => {
                    self.append(char::from(byte));
                    continue;
                }
                0xc2..=0xdf => (1, 0x1f, 0x80, 0xbf),
                0xe0 => (2, 0x0f, 0xa0, 0xbf),
                0xed => (2, 0x0f, 0x80, 0x9f),
                0xe1..=0xef => (2, 0x0f, 0x80, 0xbf),
                0xf0 => (3, 0x07, 0x90, 0xbf),
                0xf4 => (3, 0x07, 0x80, 0x8f),
                0xf1..=0xf3 => (3, 0x07, 0x80, 0xbf),
                _ => {
                    self.append(char::REPLACEMENT_CHARACTER);
                    continue;
                }
            };
            self.decoders[stream] = Decoder { remaining, cp: u32::from(byte & mask), lower, upper };
        }
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        for stream in 0..2 {
            if self.decoders[stream].remaining != 0 {
                self.append(char::REPLACEMENT_CHARACTER);
            }
        }
        self.trim();
        self.bytes
    }
}

pub fn core(t: i64, data: &[u8], chunk: i64) -> Vec<u8> {
    assert!(t > 0 && chunk > 0);
    let mut tail = Tail::new(t as usize);
    for (index, piece) in data.chunks(chunk as usize).enumerate() {
        tail.push(index % 2, piece);
    }
    tail.finish()
}
