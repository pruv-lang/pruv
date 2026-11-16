/// Representation conversion only: invalid UTF-8 returns an empty String.
/// (The entry core uses only [i64] packets; this file exists for build-script symmetry with the other pieces.)
pub fn utf8_to_string(data: &[u8]) -> String {
    std::str::from_utf8(data).map(str::to_owned).unwrap_or_default()
}
