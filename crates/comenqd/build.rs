use std::{env, fs, path::PathBuf};

fn main() {
    if let Err(error) = copy_man_page() {
        panic!("failed to stage man page: {error}");
    }
}

fn copy_man_page() -> std::io::Result<()> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source = manifest_dir.join("../../packaging/man/comenqd.1");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::create_dir_all(&out_dir)?;
    let dest = out_dir.join("comenqd.1");
    fs::copy(&source, &dest)?;
    println!("cargo:rerun-if-changed={}", source.display());
    Ok(())
}
