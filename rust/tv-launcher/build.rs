fn main() {
    // rebuild when the build stamp changes
    println!("cargo:rerun-if-env-changed=TV_BUILD");
}
