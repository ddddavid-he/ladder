/// build.rs - Compile-time mihomo binary embedding for the `bundled` feature
///
/// When built with `--features bundled`:
///   1. Detect target OS + arch from environment variables
///   2. Download matching mihomo release from GitHub
///   3. Extract the binary and write to OUT_DIR/mihomo
///   4. src/mihomo.rs uses `include_bytes!(concat!(env!("OUT_DIR"), "/mihomo"))`
///
/// This stub is a placeholder; full implementation is in P2 phase.

fn main() {
    // Inform cargo to re-run only when build.rs itself changes
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(feature = "bundled")]
    {
        // TODO P2: download_mihomo_for_target()
        panic!("bundled feature not yet implemented - P2");
    }
}
