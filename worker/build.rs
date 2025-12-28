extern crate core;

use std::{env, fs, io};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use serde::Deserialize;

fn find_workspace_root(start: &Path) -> PathBuf {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join("Cargo.toml").exists() {
            return cur;
        }
        if !cur.pop() {
            panic!("Could not find workspace Cargo.toml above {}", start.display());
        }
    }
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    filenames: Option<Vec<String>>
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=../wasm-bridge");

    let out_dir = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .expect("build script ran with no OUT_DIR environment variable");

    let cargo_path = env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| "cargo".into());

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = find_workspace_root(&manifest_dir);

    let output = Command::new(&cargo_path)
        .current_dir(&workspace_root)
        .args([
            "build",
            "-p",
            "wasm-bridge",
            "--profile",
            "wasm-bridge",
            "--target",
            "wasm32-unknown-unknown",
            "--message-format=json"
        ])
        .output()
        .expect("failed to run cargo build for bridge");

    assert!(output.status.success(), "bridge build failed");

    let json_stream = String::from_utf8(output.stdout).unwrap();

    let messages = serde_json::Deserializer::from_str(&json_stream)
        .into_iter()
        .collect::<serde_json::Result<Vec<CargoMessage>>>()?;

    let file = messages
        .iter()
        .find_map(|msg| {
            let files = msg
                .filenames
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(String::as_str)
                .filter(|path| {
                    Path::new(path).extension().map(OsStr::as_encoded_bytes) == Some(b"wasm")
                })
                .collect::<Vec<_>>();

            let reason = msg.reason.trim().to_lowercase();

            let ("compiler-artifact", Ok([file])) = (&*reason, <[_; 1]>::try_from(files)) else {
                return None
            };

            Some(Path::new(file))
        });

    let Some(file) = file else {
        panic!("could not locate wasm-bridge module")
    };

    let artifact = fs::File::open(file)?;
    let mut obj_file = fs::File::create(out_dir.join("wasm-worker-bridge.wasm.obj"))?;
    obj_file.write_all(&u64::to_le_bytes(artifact.metadata()?.len()))?;

    let mut wasm_stream = flate2::write::DeflateEncoder::new(
        obj_file,
        flate2::Compression::new(7)
    );

    io::copy(
        &mut {artifact},
        &mut wasm_stream
    )?;

    wasm_stream.finish()?.flush()
}