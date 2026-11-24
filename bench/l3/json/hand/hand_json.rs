//! A borrowed JSON source and an owned, six-slot tape.
use std::cell::Cell;

const WIDTH: usize = 6;
const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 1024 * 1024;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    // Source bytes materialized in raw results, excluding allocator relocations.
    pub bytes_copied: usize,
    // Explicit allocations and capacity growth in this module.
    pub allocations: usize,
    // Record reads by accessors, counting repeated reads.
    pub nodes_visited: usize,
}

thread_local! {
    pub static COUNTERS: Cell<Counters> = Cell::new(Counters::default());
}

fn count(update: impl FnOnce(&mut Counters)) {
    COUNTERS.with(|cell| {
        let mut counters = cell.get();
        update(&mut counters);
        cell.set(counters);
    });
}

fn output(capacity: usize) -> Vec<i64> {
    if capacity > 0 {
        count(|c| c.allocations += 1);
    }
    Vec::with_capacity(capacity)
}

fn status(value: i64) -> Vec<i64> {
    let mut out = output(1);
    out.push(value);
    out
}

type ParseResult<T> = Result<T, (usize, i64)>;

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    tape: Vec<i64>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn required(&self) -> ParseResult<u8> {
        self.peek().ok_or((self.pos, 5))
    }

    fn literal(&mut self, literal: &[u8]) -> ParseResult<()> {
        for &expected in literal {
            if self.required()? != expected {
                return Err((self.pos, 12));
            }
            self.pos += 1;
        }
        Ok(())
    }

    fn digits(&mut self) -> ParseResult<()> {
        if !self.required()?.is_ascii_digit() {
            return Err((self.pos, 13));
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        Ok(())
    }

    fn number(&mut self) -> ParseResult<()> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if self.required()? == b'0' {
            self.pos += 1;
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err((self.pos, 13));
            }
        } else {
            self.digits()?;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.digits()?;
        }
        Ok(())
    }

    fn string(&mut self) -> ParseResult<()> {
        self.pos += 1;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(());
                }
                0..=31 => return Err((self.pos, 16)),
                b'\\' => {
                    self.pos += 1;
                    match self.required()? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                        b'u' => {
                            for _ in 0..4 {
                                self.pos += 1;
                                if !self.required()?.is_ascii_hexdigit() {
                                    return Err((self.pos, 15));
                                }
                            }
                        }
                        _ => return Err((self.pos, 14)),
                    }
                }
                128..=255 => {
                    let width = utf8_width(byte);
                    let end = self.pos + width;
                    if !self.input.get(self.pos..end)
                        .is_some_and(|scalar| std::str::from_utf8(scalar).is_ok())
                    {
                        return Err((self.pos, 4));
                    }
                    self.pos = end;
                    continue;
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err((self.pos, 17))
    }

    fn record(&mut self, parent: i64, depth: usize, key: bool) -> ParseResult<()> {
        self.whitespace();
        let start = self.pos;
        let byte = self.required()?;
        if key && byte != b'"' {
            return Err((start, 7));
        }
        let kind = match byte {
            b'n' => { self.literal(b"null")?; 0 }
            b'f' => { self.literal(b"false")?; 1 }
            b't' => { self.literal(b"true")?; 2 }
            b'-' | b'0'..=b'9' => { self.number()?; 3 }
            b'"' => { self.string()?; if key { 7 } else { 4 } }
            b'[' => { self.pos += 1; 5 }
            b'{' => { self.pos += 1; 6 }
            _ => return Err((start, 6)),
        };
        let base = self.tape.len();
        let id = base / WIDTH;
        if id == MAX_RECORDS {
            return Err((start, 2));
        }
        if matches!(kind, 5 | 6) && depth == MAX_DEPTH {
            return Err((start, 3));
        }
        // The tape grows inside the append (Vec doubling), the growth point the Pruv push has.
        let capacity = self.tape.capacity();
        self.tape.extend_from_slice(&[
            kind, start as i64, self.pos as i64, (id + 1) as i64, parent, 0,
        ]);
        if self.tape.capacity() != capacity {
            count(|c| c.allocations += 1);
        }
        if matches!(kind, 5 | 6) {
            let children = self.container(id as i64, depth + 1, kind == 6)?;
            self.tape[base + 2] = self.pos as i64;
            self.tape[base + 3] = (self.tape.len() / WIDTH) as i64;
            self.tape[base + 5] = children;
        }
        Ok(())
    }

    fn container(&mut self, id: i64, depth: usize, object: bool) -> ParseResult<i64> {
        let close = if object { b'}' } else { b']' };
        self.whitespace();
        if self.peek() == Some(close) {
            self.pos += 1;
            return Ok(0);
        }
        let mut children = 0;
        loop {
            if object {
                self.record(id, depth, true)?;
                self.whitespace();
                if self.required()? != b':' {
                    return Err((self.pos, 8));
                }
                self.pos += 1;
            }
            self.record(id, depth, false)?;
            children += 1;
            self.whitespace();
            match self.required()? {
                byte if byte == close => {
                    self.pos += 1;
                    return Ok(children);
                }
                b',' => {
                    self.pos += 1;
                    self.whitespace();
                    if self.peek() == Some(close) {
                        return Err((self.pos, 10));
                    }
                }
                _ => return Err((self.pos, 9)),
            }
        }
    }
}

fn utf8_width(byte: u8) -> usize {
    match byte {
        194..=223 => 2,
        224..=239 => 3,
        240..=244 => 4,
        _ => 1,
    }
}

fn parse(input: &[u8]) -> ParseResult<Vec<i64>> {
    if input.len() > MAX_INPUT {
        return Err((0, 1));
    }
    let mut parser = Parser { input, pos: 0, tape: output(32 * WIDTH) };
    let result = parser.record(-1, 0, false).and_then(|()| {
        parser.whitespace();
        if parser.pos == input.len() { Ok(()) } else { Err((parser.pos, 11)) }
    });
    // Prior bytes have already been validated. On grammar failure, preserve
    // the ABI's priority for invalid UTF-8 anywhere in the remaining input.
    if let Err((position, kind)) = result {
        if kind != 4 {
            if let Err(error) = std::str::from_utf8(&input[position..]) {
                return Err((position + error.valid_up_to(), 4));
            }
        }
        return Err((position, kind));
    }
    Ok(parser.tape)
}

pub fn parse_tape(input: &[u8]) -> Vec<i64> {
    parse(input).unwrap_or_else(|(pos, kind)| {
        let mut out = output(3);
        out.extend([-1, pos as i64, kind]);
        out
    })
}

// The parser guarantees spans and jumps. Accessors check caller-supplied IDs.
struct Document<'a> {
    source: &'a [u8],
    tape: Vec<i64>,
}

impl Document<'_> {
    fn record(&self, id: i64) -> &[i64] {
        count(|c| c.nodes_visited += 1);
        let base = id as usize * WIDTH;
        &self.tape[base..base + WIDTH]
    }

    fn value(&self, id: i64) -> Result<&[i64], i64> {
        if id < 0 || id >= (self.tape.len() / WIDTH) as i64 {
            return Err(-3);
        }
        let record = self.record(id);
        if record[0] == 7 {
            return Err(-3);
        }
        Ok(record)
    }

    fn span(&self, record: &[i64]) -> &[u8] {
        &self.source[record[1] as usize..record[2] as usize]
    }

    fn array_at(&self, id: i64, index: i64) -> Result<i64, i64> {
        let rec = self.value(id)?;
        if rec[0] != 5 {
            return Err(-2);
        }
        if index < 0 || index >= rec[5] {
            return Err(-1);
        }
        let mut child = id + 1;
        for _ in 0..index {
            child = self.record(child)[3];
        }
        Ok(child)
    }

    fn field(&self, id: i64, matches: impl Fn(&[u8]) -> bool) -> Result<i64, i64> {
        let rec = self.value(id)?;
        if rec[0] != 6 {
            return Err(-2);
        }
        let mut found = Err(-1);
        let mut key = id + 1;
        while key < rec[3] {
            let value = key + 1;
            if matches(self.span(self.record(key))) {
                found = Ok(value); // Duplicate keys resolve to the last member.
            }
            key = self.record(value)[3];
        }
        found
    }
}

// Each escape contributes a UTF-16 code unit, including lone surrogates.
fn units(raw: &[u8]) -> impl Iterator<Item = u16> + '_ {
    body_units(&raw[1..raw.len() - 1])
}

// UTF-16 code units of a string body (no quotes).
fn body_units(mut body: &[u8]) -> impl Iterator<Item = u16> + '_ {
    let mut low = None;
    std::iter::from_fn(move || {
        if let Some(unit) = low.take() {
            return Some(unit);
        }
        let &byte = body.first()?;
        if byte == b'\\' {
            let (unit, width) = match body[1] {
                b'u' => {
                    let code = body[2..6].iter().fold(0, |n, b| {
                        n * 16 + (*b as char).to_digit(16).unwrap() as u16
                    });
                    (code, 6)
                }
                b'b' => (8, 2),
                b'f' => (12, 2),
                b'n' => (10, 2),
                b'r' => (13, 2),
                b't' => (9, 2),
                escape => (escape as u16, 2),
            };
            body = &body[width..];
            return Some(unit);
        }
        let width = utf8_width(byte);
        let scalar = std::str::from_utf8(&body[..width]).unwrap().chars().next().unwrap();
        body = &body[width..];
        let mut pair = [0; 2];
        let encoded = scalar.encode_utf16(&mut pair);
        low = encoded.get(1).copied();
        Some(encoded[0])
    })
}

// A quoted string as UTF-8 segments: each escape-free run as it is, then one char per escape
// (UTF-16 pairs joined, lone surrogates U+FFFD).
enum Piece<'a> { Run(&'a [u8]), Char(char) }

fn utf8_segments<'a>(raw: &'a [u8], mut piece: impl FnMut(Piece<'a>)) {
    let mut body = &raw[1..raw.len() - 1];
    while let Some(&byte) = body.first() {
        let width = if byte == b'\\' {
            // One scalar per step: a high surrogate escape takes the next `\u` when it is the low half.
            let ch = char::decode_utf16(body_units(body)).next().unwrap().unwrap_or(char::REPLACEMENT_CHARACTER);
            piece(Piece::Char(ch));
            match ch.len_utf16() {
                2 => 12,
                _ if body[1] == b'u' => 6,
                _ => 2,
            }
        } else {
            let width = body.iter().position(|&b| b == b'\\').unwrap_or(body.len());
            piece(Piece::Run(&body[..width]));
            width
        };
        body = &body[width..];
    }
}

fn raw_output(raw: &[u8], number: bool) -> Vec<i64> {
    let mut out = output(raw.len() + if number { 4 } else { 0 });
    if number {
        let integer = std::str::from_utf8(raw)
            .unwrap()
            .parse::<i64>()
            .ok()
            .filter(|&n| (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&n));
        out.extend([0, i64::from(integer.is_some()), integer.unwrap_or(0), i64::from(raw == b"-0")]);
    }
    out.extend(raw.iter().copied().map(i64::from));
    count(|c| c.bytes_copied += raw.len());
    out
}

pub fn query(input: &[u8], ops: &[i64], key: &[i64], text: &str, mode: i64) -> Vec<i64> {
    let Ok(tape) = parse(input) else { return status(-10); };
    let doc = Document { source: input, tape };
    let mut id = 0;
    for pair in ops.chunks_exact(2) {
        let arg = pair[1];
        let result = match pair[0] {
            0 => { id = arg; continue; }
            1 => doc.array_at(id, arg),
            2 => doc.field(id, |raw| units(raw).map(i64::from).eq(key.iter().copied())),
            3 if text.len() > MAX_INPUT => Err(-1),
            3 => doc.field(id, |raw| {
                let body = &raw[1..raw.len() - 1];
                if text.is_ascii() && !body.contains(&b'\\') {
                    body == text.as_bytes()
                } else {
                    units(raw).eq(text.encode_utf16())
                }
            }),
            _ => Ok(id),
        };
        match result {
            Ok(next) if next >= 0 => id = next,
            Ok(negative) | Err(negative) => return status(negative),
        }
    }
    let rec = match doc.value(id) {
        Ok(rec) => rec,
        Err(code) => return status(code),
    };
    let kind = rec[0];
    match mode {
        0 => status(kind),
        2 if kind == 5 => status(rec[5]),
        3 if matches!(kind, 1 | 2) => status(kind - 1),
        4 if kind == 3 => raw_output(doc.span(rec), true),
        5 if kind == 4 => {
            let raw = doc.span(rec);
            let mut out = output(units(raw).count());
            out.extend(units(raw).map(i64::from));
            out
        }
        6 if kind == 4 => {
            let raw = doc.span(rec);
            let mut need = 0;
            utf8_segments(raw, |piece| need += match piece {
                Piece::Run(run) => run.len(),
                Piece::Char(ch) => ch.len_utf8(),
            });
            let mut out = output(need);
            utf8_segments(raw, |piece| match piece {
                Piece::Run(run) => out.extend(run.iter().map(|&b| i64::from(b))),
                Piece::Char(ch) => {
                    let mut buf = [0; 4];
                    out.extend(ch.encode_utf8(&mut buf).bytes().map(i64::from));
                }
            });
            out
        }
        2..=6 => status(-2),
        _ => raw_output(doc.span(rec), false),
    }
}
