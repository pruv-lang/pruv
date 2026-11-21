// Hand-written safe-Rust baseline of read::window (bench/l3/grok/read.pruv): history window bytes -> the read result packet
// [status, 24, nrows, ntext, meta x24, rows (10 words per turn), text bytes widened to i64].
// Mounted by run_perf.py as `mod hand { include!("hand_history.rs"); }`, so this file has no inner attributes.
//
// One pass: a cursor over the window finds each LF, each line is validated and scanned once into a reusable Entry (no
// JSON document, no tape, no per-line copy), the entry goes straight to the collector (a ring of 200 slots), and the
// packet is written once into a Vec reserved at its exact size. Semantics follow framing.pruv, entry.pruv,
// bench/l3/pkgs/turns/turns.pruv and packet.pruv.
//
// Scope of the baseline (the bench inputs stay inside it; the driver checks word-for-word equality with the Pruv packet):
//  - object keys are compared by their raw spelling (no escaped keys);
//  - a timestamp counts when it is a quoted canonical 24-byte ISO string; numbers and digit strings (which isoOf would
//    convert) are treated as absent;
//  - JSON nesting deeper than 128 makes the line a skipped line.

const MAX_WIN: usize = 8388608;
const MAX_LINE: usize = 1048576;
const DROP_HEAD: usize = 8192;
const RING: usize = 200;
const CAP: usize = 32768;
const KEEP_BYTES: usize = 2097152;
const TOOLS_MAX: i64 = 1_000_000_000_000_000;

// ---------------------------------------------------------------- text helpers (text.pruv)
fn space_len(s: &[u8], i: usize) -> usize {
    if i >= s.len() {
        return 0;
    }
    let b = s[i];
    if (9..=13).contains(&b) || b == 32 {
        return 1;
    }
    if i + 1 >= s.len() {
        return 0;
    }
    let c = s[i + 1];
    if b == 194 && c == 160 {
        return 2;
    }
    if i + 2 >= s.len() {
        return 0;
    }
    let d = s[i + 2];
    if (b == 225 && c == 154 && d == 128)
        || (b == 226 && c == 128 && ((128..=138).contains(&d) || d == 168 || d == 169 || d == 175))
        || (b == 226 && c == 129 && d == 159)
        || (b == 227 && c == 128 && d == 128)
        || (b == 239 && c == 187 && d == 191)
    {
        return 3;
    }
    0
}

fn is_boundary(s: &[u8], i: usize) -> bool {
    i >= s.len() || (s[i] & 0xC0) != 0x80
}

fn is_blank(s: &[u8]) -> bool {
    let mut i = 0;
    while i < s.len() {
        let w = space_len(s, i);
        if w == 0 {
            return false;
        }
        i += w;
    }
    true
}

// Bounds of trim(s).
fn trim_span(s: &[u8]) -> (usize, usize) {
    let n = s.len();
    let mut lo = n;
    let mut i = 0;
    while i < n {
        if space_len(s, i) == 0 {
            lo = i;
            break;
        }
        i += 1;
        while i < n && !is_boundary(s, i) {
            i += 1;
        }
    }
    let mut hi = n;
    let mut k = n;
    while k > lo {
        let mut j = k - 1;
        while j > lo && !is_boundary(s, j) {
            j -= 1;
        }
        if space_len(s, j) == k - j {
            k = j;
            hi = j;
        } else {
            hi = k;
            k = lo;
        }
    }
    (lo, hi)
}

// Longest whole-scalar prefix length within max bytes.
fn clip(s: &[u8], max: usize) -> usize {
    let mut i = max.min(s.len());
    while i > 0 && !is_boundary(s, i) {
        i -= 1;
    }
    i
}

// <user_query>...</user_query> inner bounds of the trimmed span [lo, hi), else None (entry.pruv query_range).
fn query_range(s: &[u8], lo: usize, hi: usize) -> Option<(usize, usize)> {
    if hi - lo < 25 || !s[lo..].starts_with(b"<user_query>") || !s[hi - 13..].starts_with(b"</user_query>") {
        return None;
    }
    let mut a = lo + 12;
    let mut b = hi - 13;
    if a < b {
        if s[a] == 10 {
            a += 1;
        } else if s[a] == 13 && a + 1 < b && s[a + 1] == 10 {
            a += 2;
        }
    }
    if a < b && s[b - 1] == 10 {
        b -= 1;
        if a < b && s[b - 1] == 13 {
            b -= 1;
        }
    }
    Some((a, b))
}

// ---------------------------------------------------------------- dropped_user (framing.pruv)
fn skip_ws(h: &[u8], p: usize) -> usize {
    let mut q = p;
    while q < h.len() {
        let w = space_len(h, q);
        if w == 0 {
            return q;
        }
        q += w;
    }
    q
}

fn quoted_end(h: &[u8], p: usize, word: &[u8]) -> Option<usize> {
    if p >= h.len() || h[p] != b'"' {
        return None;
    }
    let s = p + 1;
    if h.len() - s < word.len() || &h[s..s + word.len()] != word {
        return None;
    }
    let e = s + word.len();
    if e >= h.len() || h[e] != b'"' {
        return None;
    }
    Some(e + 1)
}

fn colon_value(h: &[u8], e: usize) -> Option<usize> {
    let a = skip_ws(h, e);
    if a >= h.len() || h[a] != b':' {
        return None;
    }
    Some(skip_ws(h, a + 1))
}

fn synth_hit(h: &[u8], p: usize) -> bool {
    let Some(e) = quoted_end(h, p, b"synthetic_reason") else { return false };
    let Some(v) = colon_value(h, e) else { return false };
    v < h.len() && h[v] == b'"'
}

fn dropped_user(h: &[u8]) -> bool {
    let a = skip_ws(h, 0);
    if a >= h.len() || h[a] != b'{' {
        return false;
    }
    let b = skip_ws(h, a + 1);
    let Some(e) = quoted_end(h, b, b"type") else { return false };
    let Some(v) = colon_value(h, e) else { return false };
    if quoted_end(h, v, b"user").is_none() {
        return false;
    }
    !(0..h.len()).any(|p| h[p] == b'"' && synth_hit(h, p))
}

// ---------------------------------------------------------------- one scanned entry
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Missing,
    Null,
    Other,
    Str,
    Arr,
}

#[derive(Clone, Copy)]
enum Stamp {
    Missing,
    Null,
    Other,
    Canon([u8; 24]),
}

struct Entry {
    is_obj: bool,
    ty: u8, // 1 user, 2 assistant, 0 anything else
    synthetic: Kind,
    prompt_index: Kind,
    content_kind: Kind,
    content: Vec<u8>,
    scratch: Vec<u8>,
    timestamp: Stamp,
    created_at: Stamp,
    tool_calls: Option<i64>,
}

impl Entry {
    fn new() -> Entry {
        Entry {
            is_obj: false,
            ty: 0,
            synthetic: Kind::Missing,
            prompt_index: Kind::Missing,
            content_kind: Kind::Missing,
            content: Vec::new(),
            scratch: Vec::new(),
            timestamp: Stamp::Missing,
            created_at: Stamp::Missing,
            tool_calls: None,
        }
    }
    fn reset(&mut self) {
        self.is_obj = false;
        self.ty = 0;
        self.synthetic = Kind::Missing;
        self.prompt_index = Kind::Missing;
        self.content_kind = Kind::Missing;
        self.content.clear();
        self.timestamp = Stamp::Missing;
        self.created_at = Stamp::Missing;
        self.tool_calls = None;
    }
    // isoOf(timestamp ?? created_at): a missing or null timestamp falls back to created_at.
    fn at(&self) -> Option<[u8; 24]> {
        let st = match self.timestamp {
            Stamp::Missing | Stamp::Null => self.created_at,
            s => s,
        };
        match st {
            Stamp::Canon(b) => Some(b),
            _ => None,
        }
    }
}

fn leap(y: u32) -> bool {
    y % 400 == 0 || (y % 100 != 0 && y % 4 == 0)
}

fn canonical_iso(b: &[u8]) -> bool {
    if b.len() != 24 {
        return false;
    }
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'.' || b[23] != b'Z' {
        return false;
    }
    let d = |i: usize| -> Option<u32> {
        if b[i].is_ascii_digit() {
            Some((b[i] - b'0') as u32)
        } else {
            None
        }
    };
    let two = |i: usize| -> Option<u32> { Some(d(i)? * 10 + d(i + 1)?) };
    let Some(y) = (|| Some(d(0)? * 1000 + d(1)? * 100 + d(2)? * 10 + d(3)?))() else { return false };
    let (Some(m), Some(day), Some(h), Some(mi), Some(s)) = (two(5), two(8), two(11), two(14), two(17)) else { return false };
    if d(20).is_none() || d(21).is_none() || d(22).is_none() {
        return false;
    }
    if !(1..=12).contains(&m) {
        return false;
    }
    let md = match m {
        2 => {
            if leap(y) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day >= 1 && day <= md && h <= 23 && mi <= 59 && s <= 59
}

// ---------------------------------------------------------------- JSON line scanner (validates the whole line)
struct P<'a> {
    s: &'a [u8],
    i: usize,
}

fn hex4(s: &[u8], at: usize) -> Option<u32> {
    if at + 4 > s.len() {
        return None;
    }
    let mut v = 0u32;
    for k in 0..4 {
        v = v * 16 + (s[at + k] as char).to_digit(16)?;
    }
    Some(v)
}

impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn lit(&mut self, w: &[u8]) -> Option<()> {
        if self.s[self.i..].starts_with(w) {
            self.i += w.len();
            Some(())
        } else {
            None
        }
    }

    // A string at i (the opening quote); returns the raw range between the quotes.
    fn skip_string(&mut self) -> Option<(usize, usize)> {
        let s = self.s;
        if s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let start = self.i;
        loop {
            let c = *s.get(self.i)?;
            match c {
                b'"' => {
                    self.i += 1;
                    return Some((start, self.i - 1));
                }
                b'\\' => match *s.get(self.i + 1)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.i += 2,
                    b'u' => {
                        hex4(s, self.i + 2)?;
                        self.i += 6;
                    }
                    _ => return None,
                },
                0..=0x1f => return None,
                _ => self.i += 1,
            }
        }
    }

    // A string at i decoded into out (lone surrogates become U+FFFD).
    fn decode_string(&mut self, out: &mut Vec<u8>) -> Option<()> {
        let s = self.s;
        if s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        loop {
            let run = self.i;
            while self.i < s.len() && s[self.i] != b'"' && s[self.i] != b'\\' && s[self.i] >= 0x20 {
                self.i += 1;
            }
            out.extend_from_slice(&s[run..self.i]);
            match *s.get(self.i)? {
                b'"' => {
                    self.i += 1;
                    return Some(());
                }
                b'\\' => {
                    let e = *s.get(self.i + 1)?;
                    self.i += 2;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(10),
                        b'r' => out.push(13),
                        b't' => out.push(9),
                        b'u' => {
                            let mut v = hex4(s, self.i)?;
                            self.i += 4;
                            if (0xD800..=0xDBFF).contains(&v) {
                                if s[self.i..].starts_with(b"\\u") {
                                    if let Some(lo) = hex4(s, self.i + 2) {
                                        if (0xDC00..=0xDFFF).contains(&lo) {
                                            v = 0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00);
                                            self.i += 6;
                                        }
                                    }
                                }
                            }
                            let ch = char::from_u32(v).unwrap_or('\u{FFFD}');
                            let mut tmp = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                        }
                        _ => return None,
                    }
                }
                _ => return None, // a raw control byte
            }
        }
    }

    fn number(&mut self) -> Option<()> {
        let s = self.s;
        if s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match *s.get(self.i)? {
            b'0' => self.i += 1,
            b'1'..=b'9' => {
                while self.i < s.len() && s[self.i].is_ascii_digit() {
                    self.i += 1;
                }
            }
            _ => return None,
        }
        if s.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !s.get(self.i)?.is_ascii_digit() {
                return None;
            }
            while self.i < s.len() && s[self.i].is_ascii_digit() {
                self.i += 1;
            }
        }
        if matches!(s.get(self.i), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(s.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if !s.get(self.i)?.is_ascii_digit() {
                return None;
            }
            while self.i < s.len() && s[self.i].is_ascii_digit() {
                self.i += 1;
            }
        }
        Some(())
    }

    // Skips one value (validating it); the kind of the value.
    fn skip_value(&mut self, depth: usize) -> Option<Kind> {
        if depth > 128 {
            return None;
        }
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Some(Kind::Other);
                }
                loop {
                    self.ws();
                    self.skip_string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return None;
                    }
                    self.i += 1;
                    self.skip_value(depth + 1)?;
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Some(Kind::Other);
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.skip_array(depth).map(|_| Kind::Arr)
            }
            b'"' => self.skip_string().map(|_| Kind::Str),
            b't' => self.lit(b"true").map(|_| Kind::Other),
            b'f' => self.lit(b"false").map(|_| Kind::Other),
            b'n' => self.lit(b"null").map(|_| Kind::Null),
            b'-' | b'0'..=b'9' => self.number().map(|_| Kind::Other),
            _ => None,
        }
    }

    // An array at i; its element count.
    fn skip_array(&mut self, depth: usize) -> Option<i64> {
        self.i += 1;
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Some(0);
        }
        let mut n = 0;
        loop {
            self.skip_value(depth + 1)?;
            n += 1;
            self.ws();
            match *self.s.get(self.i)? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(n);
                }
                _ => return None,
            }
        }
    }

    // Object members: calls f(self, key range) positioned at the value (whitespace skipped); f consumes the value.
    fn members<F: FnMut(&mut P<'a>, &'a [u8]) -> Option<()>>(&mut self, mut f: F) -> Option<()> {
        let s = self.s;
        self.i += 1; // '{'
        self.ws();
        if s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Some(());
        }
        loop {
            self.ws();
            let (ks, ke) = self.skip_string()?;
            self.ws();
            if s.get(self.i) != Some(&b':') {
                return None;
            }
            self.i += 1;
            self.ws();
            f(self, &s[ks..ke])?;
            self.ws();
            match *s.get(self.i)? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn stamp(&mut self) -> Option<Stamp> {
        if self.s.get(self.i) == Some(&b'"') {
            let (a, b) = self.skip_string()?;
            if b - a == 24 && canonical_iso(&self.s[a..b]) {
                let mut t = [0u8; 24];
                t.copy_from_slice(&self.s[a..b]);
                return Some(Stamp::Canon(t));
            }
            return Some(Stamp::Other);
        }
        Some(match self.skip_value(1)? {
            Kind::Null => Stamp::Null,
            _ => Stamp::Other,
        })
    }

    // One element of a content array: an object's "text" string (the last one wins), joined with a blank line.
    fn part(&mut self, e: &mut Entry) -> Option<()> {
        let mut has = false;
        let mut scratch = std::mem::take(&mut e.scratch);
        let r = self.members(|p, key| {
            if key == b"text" && p.s.get(p.i) == Some(&b'"') {
                scratch.clear();
                p.decode_string(&mut scratch)?;
                has = true;
            } else {
                if key == b"text" {
                    has = false;
                }
                p.skip_value(2)?;
            }
            Some(())
        });
        if r.is_some() && has && !scratch.is_empty() {
            if !e.content.is_empty() {
                e.content.extend_from_slice(b"\n\n");
            }
            e.content.extend_from_slice(&scratch);
        }
        e.scratch = scratch;
        r
    }

    fn content(&mut self, e: &mut Entry) -> Option<()> {
        e.content.clear();
        match *self.s.get(self.i)? {
            b'"' => {
                self.decode_string(&mut e.content)?;
                e.content_kind = Kind::Str;
            }
            b'[' => {
                e.content_kind = Kind::Arr;
                self.i += 1;
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Some(());
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) == Some(&b'{') {
                        self.part(e)?;
                    } else {
                        self.skip_value(2)?;
                    }
                    self.ws();
                    match *self.s.get(self.i)? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Some(());
                        }
                        _ => return None,
                    }
                }
            }
            _ => {
                e.content_kind = self.skip_value(1)?;
            }
        }
        Some(())
    }

    // The whole line into e; None = the line is not valid JSON (a skipped line).
    fn entry(&mut self, e: &mut Entry) -> Option<()> {
        self.ws();
        if self.s.get(self.i) != Some(&b'{') {
            self.skip_value(0)?;
        } else {
            e.is_obj = true;
            self.members(|p, key| {
                match key {
                    b"type" => {
                        if p.s.get(p.i) == Some(&b'"') {
                            let (a, b) = p.skip_string()?;
                            e.ty = match &p.s[a..b] {
                                b"user" => 1,
                                b"assistant" => 2,
                                _ => 0,
                            };
                        } else {
                            p.skip_value(1)?;
                            e.ty = 0;
                        }
                    }
                    b"synthetic_reason" => e.synthetic = p.skip_value(1)?,
                    b"prompt_index" => e.prompt_index = p.skip_value(1)?,
                    b"content" => p.content(e)?,
                    b"timestamp" => e.timestamp = p.stamp()?,
                    b"created_at" => e.created_at = p.stamp()?,
                    b"tool_calls" => {
                        if p.s.get(p.i) == Some(&b'[') {
                            e.tool_calls = Some(p.skip_array(1)?);
                        } else {
                            p.skip_value(1)?;
                            e.tool_calls = None;
                        }
                    }
                    _ => {
                        p.skip_value(1)?;
                    }
                }
                Some(())
            })?;
        }
        self.ws();
        if self.i == self.s.len() {
            Some(())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- collector (turns.pruv)
#[derive(Default)]
struct Slot {
    prompt: Vec<u8>,
    reply: Vec<u8>,
    budget: usize,
    tools: i64,
    clipped: bool,
    created: Option<[u8; 24]>,
    completed: Option<[u8; 24]>,
}

struct Collector {
    slots: Vec<Slot>,
    begun: usize,
    current: bool,
    lost_before: bool,
    newest: usize,
}

impl Collector {
    fn new() -> Collector {
        Collector { slots: (0..RING).map(|_| Slot::default()).collect(), begun: 0, current: false, lost_before: false, newest: 0 }
    }

    fn begin(&mut self, prompt: &[u8], created: Option<[u8; 24]>) {
        let w = self.begun % RING;
        let n = clip(prompt, CAP);
        let clipped = prompt.len() > CAP || self.lost_before;
        let s = &mut self.slots[w];
        s.prompt.clear();
        s.prompt.extend_from_slice(&prompt[..n]);
        s.reply.clear();
        s.budget = 0;
        s.tools = 0;
        s.clipped = clipped;
        s.created = created;
        s.completed = None;
        self.begun += 1;
        self.current = true;
        self.lost_before = false;
        self.newest = w;
    }

    fn skip_user(&mut self) {
        self.current = false;
        self.lost_before = true;
    }

    fn tool(&mut self, n: i64) {
        if self.current {
            let s = &mut self.slots[self.newest];
            s.tools = (s.tools + n).min(TOOLS_MAX);
        }
    }

    fn answer(&mut self, text: &[u8], at: Option<[u8; 24]>) {
        if !self.current {
            return;
        }
        let s = &mut self.slots[self.newest];
        if at.is_some() {
            s.completed = at;
        }
        if text.is_empty() || s.budget >= CAP {
            return;
        }
        let room = CAP - s.budget;
        let sep = if s.reply.is_empty() { 0 } else { 2 };
        if sep + text.len() <= room {
            s.reply.extend_from_slice(&b"\n\n"[..sep]);
            s.reply.extend_from_slice(text);
            s.budget += sep + text.len();
            return;
        }
        let (s2, take) = if sep == 2 && room < 2 { (room, 0) } else { (sep, clip(text, room - sep)) };
        s.reply.extend_from_slice(&b"\n\n"[..s2]);
        s.reply.extend_from_slice(&text[..take]);
        s.budget = CAP;
        s.clipped = true;
    }

    // One parsed entry (entry.pruv feed).
    fn feed(&mut self, e: &Entry) {
        if !e.is_obj {
            return;
        }
        let txt: &[u8] = &e.content;
        if e.ty == 1 {
            if !matches!(e.synthetic, Kind::Missing | Kind::Null) {
                return;
            }
            let (lo, hi) = trim_span(txt);
            let prompt = match query_range(txt, lo, hi) {
                Some((a, b)) => &txt[a..b],
                None => {
                    if matches!(e.prompt_index, Kind::Missing | Kind::Null) && txt[lo..].starts_with(b"<user_info>") {
                        return;
                    }
                    txt
                }
            };
            if is_blank(prompt) {
                return;
            }
            self.begin(prompt, e.at());
        } else if e.ty == 2 {
            self.answer(txt, e.at());
            if let Some(n) = e.tool_calls {
                self.tool(n);
            }
        }
    }

    // The newest suffix of the ring (at most 2097152 prompt+reply bytes) as the read packet, oldest turn first.
    fn packet(&self, omitted: bool, title: &str, model: &str) -> Vec<i64> {
        let avail = self.begun.min(RING);
        let (mut kept, mut total, mut tl) = (0usize, 0usize, 0usize);
        while kept < avail {
            let s = &self.slots[(self.begun - 1 - kept) % RING];
            let size = s.prompt.len() + s.reply.len();
            if total + size > KEEP_BYTES {
                break;
            }
            total += size;
            tl += size + 24 * s.created.is_some() as usize + 24 * s.completed.is_some() as usize;
            kept += 1;
        }
        let nr = 10 * kept;
        let nx = tl + title.len() + model.len();
        let mut p: Vec<i64> = Vec::with_capacity(28 + nr + nx);
        p.resize(28, 0);
        let mut lo = 0usize;
        for j in (0..kept).rev() {
            let s = &self.slots[(self.begun - 1 - j) % RING];
            let (pl, rl) = (s.prompt.len(), s.reply.len());
            let cp = s.created.is_some() as usize;
            let dp = s.completed.is_some() as usize;
            let c0 = lo + pl + rl;
            let d0 = c0 + 24 * cp;
            p.extend_from_slice(&[
                lo as i64,
                pl as i64,
                (lo + pl) as i64,
                rl as i64,
                if cp == 1 { c0 as i64 } else { -1 },
                24 * cp as i64,
                if dp == 1 { d0 as i64 } else { -1 },
                24 * dp as i64,
                s.tools,
                s.clipped as i64,
            ]);
            lo = d0 + 24 * dp;
        }
        for j in (0..kept).rev() {
            let s = &self.slots[(self.begun - 1 - j) % RING];
            p.extend(s.prompt.iter().map(|&b| b as i64));
            p.extend(s.reply.iter().map(|&b| b as i64));
            if let Some(c) = &s.created {
                p.extend(c.iter().map(|&b| b as i64));
            }
            if let Some(c) = &s.completed {
                p.extend(c.iter().map(|&b| b as i64));
            }
        }
        p.extend(title.bytes().map(|b| b as i64));
        p.extend(model.bytes().map(|b| b as i64));
        p[0] = 0;
        p[1] = 24;
        p[2] = nr as i64;
        p[3] = nx as i64; // turn text + title + model
        // meta (packet.pruv meta_new(1) + assemble's edits)
        let m = &mut p[4..28];
        m[0] = 2;
        m[1] = 1;
        for k in [4, 6, 8, 18, 20, 22] {
            m[k] = -1;
        }
        if omitted || kept < self.begun {
            m[2] = 1;
        }
        m[3] = kept as i64;
        m[10] = 10;
        if !title.is_empty() {
            m[4] = tl as i64;
            m[5] = title.len() as i64;
        }
        if !model.is_empty() {
            m[6] = (tl + title.len()) as i64;
            m[7] = model.len() as i64;
        }
        p
    }
}

fn simple_error(stage: i64, cls: i64, code: i64) -> Vec<i64> {
    let mut p = vec![0i64; 28];
    p[0] = 1;
    p[1] = 24;
    p[4] = 2;
    p[5] = 1;
    for k in [8, 10, 12, 22, 24, 26] {
        p[k] = -1;
    }
    p[17] = stage;
    p[18] = cls;
    p[19] = code;
    p
}

// ---------------------------------------------------------------- the window
pub fn history_window(win: &[u8], omitted: bool, title: &str, model: &str) -> Vec<i64> {
    if win.len() > MAX_WIN {
        return simple_error(12, 6, 1);
    }
    if title.len() > 1_000_000 || model.len() > 20_000_000 {
        return simple_error(12, 6, 2);
    }
    let n = win.len();
    let mut col = Collector::new();
    let mut entry = Entry::new();
    let lf = |from: usize| win[from..].iter().position(|&b| b == b'\n').map(|k| from + k);
    let mut p = if omitted { lf(0).map_or(n, |q| q + 1) } else { 0 };
    while p < n {
        match lf(p) {
            None => {
                if n - p > MAX_LINE && dropped_user(&win[p..p + DROP_HEAD]) {
                    col.skip_user();
                }
                p = n;
            }
            Some(q) => {
                let ln = q - p;
                if ln > MAX_LINE {
                    if dropped_user(&win[p..p + DROP_HEAD]) {
                        col.skip_user();
                    }
                } else if ln > 0 {
                    let line = &win[p..q];
                    if std::str::from_utf8(line).is_ok() {
                        entry.reset();
                        if (P { s: line, i: 0 }).entry(&mut entry).is_some() {
                            col.feed(&entry);
                        }
                    }
                }
                p = q + 1;
            }
        }
    }
    col.packet(omitted, title, model)
}
