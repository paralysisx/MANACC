use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

fn main() {
    bake_riot_key();
    let attrs = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new()
            .app_manifest(include_str!("windows-manifest.xml")),
    );
    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}

/// Builds the Riot API key into the app when the build machine provides one in
/// RIOT_API_KEY (a repository secret in the release workflow). The key is never in
/// the repository. It is stored masked with random bytes, so it does not appear as
/// text in the program file. That stops a text search, not someone who inspects the
/// running app: if the key is ever misused, regenerate it and release again.
fn bake_riot_key() {
    println!("cargo:rerun-if-env-changed=RIOT_API_KEY");
    let key = std::env::var("RIOT_API_KEY").unwrap_or_default();
    let key = key.trim();
    let code = if key.is_empty() {
        "pub const KEY: Option<(&[u8], &[u8])> = None;\n".to_string()
    } else {
        let mask = random_bytes(key.len());
        let masked: Vec<u8> = key.bytes().zip(&mask).map(|(byte, mask)| byte ^ mask).collect();
        format!("pub const KEY: Option<(&[u8], &[u8])> = Some((&{masked:?}, &{mask:?}));\n")
    };
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("baked_key.rs");
    std::fs::write(out, code).expect("failed to write baked_key.rs");
}

fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count + 8);
    while bytes.len() < count {
        bytes.extend(RandomState::new().build_hasher().finish().to_le_bytes());
    }
    bytes.truncate(count);
    bytes
}
