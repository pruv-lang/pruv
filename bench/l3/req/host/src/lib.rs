// Thin library entry of the L3-9 request host (crate `req_host`): the leaf wrapper reuses the L3-8 entry host
// (bench/l3/entry/host.rs, unchanged) inside module `eh`; the request driver lives in bench/l3/req/host.rs.
// Included files use only plain `//` comments (include! forbids inner doc comments and inner attributes).
#[allow(dead_code)]
mod eh {
    include!("../../../entry/host.rs");
    include!("../../leaf.rs");
}
include!("../../host.rs");
