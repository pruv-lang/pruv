// Typed boundary for the shared M9 scheduler and benchmark workloads.
include!("driver_m9.rs");

// Encode each UTF-16 code unit independently, including lone surrogates.
const SURROGATE_OFFSET: u32 = 0x10000 - 0xd800;

#[test]
fn churn_order() {
    let mut st = World::<false>::new_state();
    assert_eq!(sol::t_put(&mut st, "main", "keep", 0, true, 0, 0, 0, 0), 0);
    assert_eq!(sol::t_put(&mut st, "other", "drop", 0, true, 0, 0, 0, 0), 0);
    let call = |st: &mut sol::St, code, sid, bid, run, waiting| {
        assert_eq!(sol::t_call(st, code, sid, bid, run, waiting, 0).status, 0);
    };
    // Leave a hole before both surviving ordinals so compaction must rewrite them.
    call(&mut st, 1, "other", "drop", "b", false);
    call(&mut st, 1, "main", "keep", "b", false);
    call(&mut st, 1, "main", "keep", "a", false);
    call(&mut st, 2, "main", "keep", "a", true);
    for _ in 0..1_000 {
        call(&mut st, 3, "main", "keep", "b", false);
        call(&mut st, 5, "other", "drop", "", false);
        let clock = &st.clocks["keep"];
        assert!(clock.order.len() <= 2 * clock.runs.len() + 16);
        assert!(st.clock_order.len() <= 2 * st.clocks.len() + 16);
        assert_eq!(clock.runs["a"] % 2, 1);
        assert_eq!(clock.order[(clock.runs["a"] / 2) as usize], "a");
        assert_eq!(st.clock_order[clock.ordinal as usize], "keep");
        assert_eq!(sol::t_i2(&mut st), 0);
        call(&mut st, 1, "main", "keep", "b", false);
        call(&mut st, 1, "other", "drop", "b", false);
    }
    let mut obs = sol::t_observe(&mut st);
    assert_eq!(obs.keys.drain(4..).map(|key| key.s).collect::<Vec<_>>(),
               ["keep", "main", "a", "b", "drop", "other", "b"]);
}

fn take_typed_key(input: &mut &[i64], key: &mut String) -> Option<()> {
    let (&n, rest) = input.split_first()?;
    if !(0..=1 << 30).contains(&n) {
        return None;
    }
    let n = n as usize;
    let packed = rest.get(..n.div_ceil(4))?;
    let padding = (4 - n % 4) % 4;
    if padding != 0 && (*packed.last()? as u64 & ((1 << (16 * padding)) - 1)) != 0 {
        return None;
    }
    key.clear();
    key.reserve(n);
    for i in 0..n {
        let unit = ((packed[i / 4] as u64 >> (16 * (3 - i % 4))) & 0xffff) as u32;
        key.push(char::from_u32(unit).unwrap_or_else(|| char::from_u32(unit + SURROGATE_OFFSET).unwrap()));
    }
    *input = &rest[packed.len()..];
    Some(())
}

fn typed_units(s: &str) -> impl Iterator<Item = i64> + '_ {
    s.chars().map(|c| {
        let scalar = c as u32;
        i64::from(if scalar < 0x10000 { scalar } else { scalar - SURROGATE_OFFSET })
    })
}

impl<const CHECK_I2: bool> World<CHECK_I2> {
    fn new_state() -> sol::St {
        sol::St {
            sessions: HashMap::new(),
            session_order: Vec::new(),
            clocks: HashMap::new(),
            clock_order: Vec::new(),
        }
    }

    fn put(&mut self, mut op: &[i64]) {
        let [sid, bid, _] = &mut self.keys;
        let parsed = (|| {
            take_typed_key(&mut op, sid)?;
            let (&n, record) = op.split_first()?;
            if n != record.len() as i64 {
                return None;
            }
            let &[state, unlimited, minutes, active_ms, since_kind, since, ref tail @ ..] = record else {
                return None;
            };
            let mut tail = tail;
            take_typed_key(&mut tail, bid)?;
            tail.is_empty().then(|| sol::t_put(
                &mut self.st, sid, bid, state, unlimited == 1, minutes, active_ms, since_kind, since,
            ))
        })();
        if CHECK_I2 {
            self.check_i2();
        }
        if parsed != Some(0) {
            eprintln!("typed t_put status {}", parsed.unwrap_or(2));
            std::process::exit(2);
        }
    }

    fn invoke(&mut self, op: &[i64]) -> i64 {
        self.fx[0] = 0;
        let [sid, bid, run] = &mut self.keys;
        let result = (|| {
            let (&code, mut rest) = op.split_first()?;
            take_typed_key(&mut rest, sid)?;
            take_typed_key(&mut rest, bid)?;
            take_typed_key(&mut rest, run)?;
            let &[flag] = rest else { return None };
            Some(sol::t_call(&mut self.st, code, sid, bid, run, flag == 1, self.now))
        })();
        let Some(reply) = result else { return 2 };
        let mut i = 1;
        for effect in reply.effects {
            self.fx[i] = effect.tag;
            i += 1;
            if effect.tag == 12 {
                self.fx[i] = effect.requested;
                self.fx[i + 1] = effect.effective;
                i += 2;
            }
        }
        self.fx[0] = (i - 1) as i64;
        if CHECK_I2 {
            self.check_i2();
        }
        reply.status
    }

    fn i2(&mut self) -> i64 {
        sol::t_i2(&mut self.st)
    }

    fn observation(&mut self) -> Result<&[i64], i64> {
        let reply = sol::t_observe(&mut self.st);
        let mut keys = reply.keys.into_iter().peekable();
        let out = &mut self.out;
        out.clear();
        for (i, word) in reply.words.into_iter().map(Some).chain([None]).enumerate() {
            while keys.peek().is_some_and(|key| key.offset == i as i64) {
                let key = keys.next().unwrap();
                if key.packed {
                    let units: Vec<_> = typed_units(&key.s).collect();
                    // The record length precedes the six scalar budget fields.
                    let record_size = out.len() - 7;
                    out[record_size] = 7 + units.len().div_ceil(4) as i64;
                    out.push(units.len() as i64);
                    pack_into(&units, out);
                } else {
                    out.push(key.s.chars().count() as i64);
                    out.extend(typed_units(&key.s));
                }
            }
            out.extend(word);
        }
        assert!(keys.next().is_none(), "unordered observation key offsets");
        Ok(out)
    }

    fn live(&self) -> (i64, usize) {
        let entries = 3 + self.st.sessions.len() + self.st.clocks.len()
            + self.st.clocks.values().map(|clock| clock.runs.len()).sum::<usize>();
        (entries as i64, 0)
    }
}
