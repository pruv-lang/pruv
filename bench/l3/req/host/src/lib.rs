// Thin library entry of the L3-9 request host (crate `req_host`): the leaf wrapper reuses the L3-8 entry host
// (bench/l3/entry/host.rs) and its packed pool decoder inside module `eh`; the request driver lives in bench/l3/req/host.rs.
// Included files use only plain `//` comments (include! forbids inner doc comments and inner attributes).
#[allow(dead_code)]
mod eh {
    include!("../../../entry/host.rs");
    include!("../../leaf.rs");

    // D-DB20-2: the packet pool, decoded at the first string read and then reused for the rest of the packet.
    struct LazyPool<'a> {
        sec: &'a [i64],
        cell: std::cell::OnceCell<Result<Pool<'a>, String>>,
    }

    impl<'a> LazyPool<'a> {
        fn new(sec: &'a [i64]) -> Self {
            LazyPool { sec, cell: std::cell::OnceCell::new() }
        }
    }

    fn units_at(pool: &LazyPool<'_>, off: i64, len: i64) -> Result<Vec<u16>, String> {
        match pool.cell.get_or_init(|| decode_pool(pool.sec).map_err(he)) {
            Ok(p) => p.string_units(off, len).map_err(he),
            Err(e) => Err(e.clone()),
        }
    }

    pub(super) fn folder_units(pool: &[i64], rec: &[i64]) -> Result<Vec<u16>, String> {
        let units = match rec.get(27..30) {
            Some([6, off, len]) => units_at(&LazyPool::new(pool), *off, *len).ok(),
            _ => None,
        };
        units.ok_or_else(|| "internal: checkFolder await without a string folder slot".to_string())
    }
}
include!("../../host.rs");
