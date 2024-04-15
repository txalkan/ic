fn main() {
    let did_path = std::path::PathBuf::from("minter.did")
        .canonicalize()
        .unwrap();

    println!(
        "cargo:rustc-env=MINTER_DID_PATH={}",
        did_path.display()
    );
}
