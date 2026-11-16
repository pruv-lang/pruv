# L3-3 MCP stdio check

Run from the repository root:

```
PATH="$HOME/.cargo/bin:$PATH" python3 bench/l3/mcp/test_impl.py
```

The driver verifies and compiles root.pruv with both compilers and both backends,
builds the Rust renderer/fingerprint adapter offline, and starts only local Python fixture children.
It does not run other suites. Build products stay in work/.

Dependencies: the repository's compiler/solver installations, Rust, clang,
cached serde_json Cargo dependencies, Python 3 and Node. MCP_NODE overrides the
Node binary (the test environment defaults to /opt/homebrew/bin/node).
MCP_FINGERPRINT_SCRIPT points the standalone harness to fingerprint.cjs.

The harness reads JSON requests on stdin (command, args array, cwd, timeout in
milliseconds) and writes one JSON result per request. Default timeout is 15000.
See docs/MCP_CHECK_SPEC.md for the packet, trusted boundary and differences.

After a host-only edit, `--reuse-verified` rebuilds the host/harness and reruns
fixtures using the previously verified Pruv emissions. Use it only when no Pruv
source changed since the normal run; it does not verify the sources again.

Transport is implemented in root.pruv and transport.pruv using G10 Process/Loop.
G10 reports OS launch failures as code 2. A small host filesystem lookup maps
missing commands to ENOENT; its race and platform limits are documented in the spec.
The Node fingerprint invocation lives in the harness, outside host/.
The driver continues after assertion failures to report every pair.
