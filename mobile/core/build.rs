//! With the `mercury` feature, link Rhizomatica's Mercury modem (built by
//! mobile/build-mercury.sh) into the library. MERCURY_LIB_DIR points at
//! mobile/build/mercury/<target>/lib; by default it's found from the target.
fn main() {
    println!("cargo:rerun-if-env-changed=MERCURY_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_MERCURY").is_none() {
        return;
    }
    let target = std::env::var("TARGET").unwrap();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let dir = std::env::var("MERCURY_LIB_DIR").unwrap_or_else(|_| {
        let t = match target.as_str() {
            "aarch64-apple-ios-sim" => "ios-sim",
            "aarch64-apple-ios" => "ios",
            "aarch64-linux-android" => "android-arm64",
            t => panic!("no Mercury build for {t}: run mobile/build-mercury.sh"),
        };
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        format!("{manifest}/../build/mercury/{t}/lib")
    });
    println!("cargo:rerun-if-changed={dir}");
    println!("cargo:rustc-link-search=native={dir}");
    // Order matters for the static linker: the core first, then what it uses.
    println!("cargo:rustc-link-lib=static=mercury_core");
    println!("cargo:rustc-link-lib=static=freedvdata");
    // Mercury's audioio.a, copied as libaudioio.a by build-mercury.sh.
    println!("cargo:rustc-link-lib=static=audioio");
    match os.as_str() {
        "android" => {
            println!("cargo:rustc-link-lib=static=hamlib");
            for l in ["aaudio", "log", "dl", "m", "android"] {
                println!("cargo:rustc-link-lib={l}");
            }
            // Hamlib is C++.
            println!("cargo:rustc-link-lib=c++_static");
            println!("cargo:rustc-link-lib=c++abi");
        }
        "ios" => {
            for f in ["AudioToolbox", "AVFoundation", "Foundation", "CoreFoundation"] {
                println!("cargo:rustc-link-lib=framework={f}");
            }
        }
        _ => {}
    }
}
