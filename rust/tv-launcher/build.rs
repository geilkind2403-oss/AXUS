fn main() {
    // rebuild when the build stamp changes
    println!("cargo:rerun-if-env-changed=TV_BUILD");

    // The Weather app's animated sky (C++). Built freestanding: no C++
    // runtime, exceptions or RTTI, so the launcher stays one static binary
    // with no new library; it only uses malloc/free/memcpy from the libc
    // that is linked anyway.
    println!("cargo:rerun-if-changed=cpp/weather_fx.cpp");
    println!("cargo:rerun-if-changed=cpp/weather_fx.h");
    // ... and the fast menu's liquid glass
    println!("cargo:rerun-if-changed=cpp/glass_fx.cpp");
    println!("cargo:rerun-if-changed=cpp/glass_fx.h");
    let target = std::env::var("TARGET").unwrap_or_default();
    let mut b = cc::Build::new();
    b.cpp(true)
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
        b.flag("-mcpu=cortex-a53")
            .flag("-mfpu=neon-fp-armv8")
            .flag("-mfloat-abi=hard");
    }
    b.compile("tvfx");
}
