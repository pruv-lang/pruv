// Thin library entry of the L3-10 budget host (crate `budget_host`): the world mechanics live in bench/l3/budget/host.rs,
// state buffers in bench/l3/budget/state_buf.rs, the JSON / JV codec in bench/l3/budget/leaf.rs. Included files use only plain `//` comments
// (include! forbids inner doc comments and inner attributes).
#![allow(dead_code)]
include!("../../state_buf.rs");
include!("../../host.rs");
include!("../../leaf.rs");
