fn main() {
    // rebuild when the build stamp changes
    println!("cargo:rerun-if-env-changed=TV_BUILD");
    let target = std::env::var("TARGET").unwrap_or_default();

    // The Weather app's animated sky and the fast menu's liquid glass (C++).
    // Built freestanding: no C++ runtime, exceptions or RTTI, so the
    // launcher stays one static binary with no new library; they only use
    // malloc/free/memcpy from the libc that is linked anyway.
    println!("cargo:rerun-if-changed=cpp/weather_fx.cpp");
    println!("cargo:rerun-if-changed=cpp/weather_fx.h");
    println!("cargo:rerun-if-changed=cpp/glass_fx.cpp");
    println!("cargo:rerun-if-changed=cpp/glass_fx.h");
    let mut fx = cc::Build::new();
    fx.cpp(true)
        .cpp_link_stdlib(None)
        .file("cpp/weather_fx.cpp")
        .file("cpp/glass_fx.cpp")
        .include("cpp")
        // the effects run every frame: optimised even in debug builds
        .opt_level(3)
        .debug(false)
        .warnings(true)
        .flag_if_supported("-std=c++17")
        .flag("-ffreestanding")
        .flag("-nostdinc")
        .flag("-fno-exceptions")
        .flag("-fno-rtti")
        .flag_if_supported("-fno-threadsafe-statics")
        .flag_if_supported("-fno-math-errno")
        .flag_if_supported("-fno-asynchronous-unwind-tables")
        .flag_if_supported("-nostdinc++");
    if target.starts_with("armv7") {
        // Raspberry Pi 3 (Cortex-A53, 32-bit userland, hard float)
        fx.flag("-mcpu=cortex-a53")
            .flag("-mfpu=neon-fp-armv8")
            .flag("-mfloat-abi=hard");
    }
    fx.compile("tvfx");

    // The Settings view (C++ presentation layer).
    println!("cargo:rerun-if-changed=src/ui/native/settings_view.cpp");
    let mut cpp = cc::Build::new();
    cpp.cpp(true)
        .file("src/ui/native/settings_view.cpp")
        .flag_if_supported("-std=c++17")
        .flag("-ffreestanding")
        .flag("-nostdinc")
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
