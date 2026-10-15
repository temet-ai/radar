//! The runtime the binary carries: when `RADAR_RUNTIME_EMBED` names the
//! executable scripts/build-runtime made, src/main.rs embeds it and writes it
//! to the cache folder when radar starts (radar_core::runtime). Unset (test and
//! development builds), nothing is embedded and radar finds `node` and the
//! bundles as before.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(radar_runtime)");
    println!("cargo::rerun-if-env-changed=RADAR_RUNTIME_EMBED");
    let Some(named) = std::env::var_os("RADAR_RUNTIME_EMBED").filter(|p| !p.is_empty()) else {
        return;
    };
    let path = std::fs::canonicalize(&named)
        .unwrap_or_else(|e| panic!("RADAR_RUNTIME_EMBED={}: {e}", Path::new(&named).display()));
    println!("cargo::rerun-if-changed={}", path.display());
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // FNV-1a of the bytes names the runtime's cache folder, so a changed
    // runtime is written out again.
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    println!("cargo::rustc-cfg=radar_runtime");
    println!("cargo::rustc-env=RADAR_RUNTIME_BLOB={}", path.display());
    println!("cargo::rustc-env=RADAR_RUNTIME_ID={}-{hash:016x}", std::env::var("CARGO_PKG_VERSION").unwrap());
}
