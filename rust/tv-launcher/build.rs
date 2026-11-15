fn main() {
    // rebuild when the build stamp changes
    println!("cargo:rerun-if-env-changed=TV_BUILD");
    println!("cargo:rerun-if-changed=src/ui/native/settings_view.cpp");
    let target = std::env::var("TARGET").unwrap_or_default();
    let mut cpp = cc::Build::new();
    cpp.cpp(true)
        .file("src/ui/native/settings_view.cpp")
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-O2")
        .flag_if_supported("-fno-exceptions")
        .flag_if_supported("-fno-rtti")
        .cpp_link_stdlib(None);
    if target == "armv7-unknown-linux-musleabihf" {
        // build-pi.sh requires clang; this translation unit uses no sysroot
        // headers beyond the C ABI and has no C++ runtime dependency.
        cpp.compiler("clang++")
            .flag("--target=armv7-unknown-linux-musleabihf");
    }
    cpp.compile("tv_settings_view");
}
