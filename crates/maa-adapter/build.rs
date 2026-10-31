fn main() {
    assert_eq!(std::env::var("TARGET").unwrap(), "x86_64-pc-windows-msvc");
    println!("cargo:rerun-if-changed=native");
    cc::Build::new()
        .cpp(true)
        .file("native/combat_template.cpp")
        .include("native/opencv/config")
        .include("native/opencv/include")
        .std("c++17")
        .define("_ITERATOR_DEBUG_LEVEL", "0")
        .static_crt(false)
        .debug(false)
        .opt_level(2)
        .flag("/EHsc")
        .flag("/utf-8")
        .warnings(false)
        .compile("zzz_combat_template");
}
