//! Compiles the C++ shim and links the OCCT shared libraries.
//!
//! OCCT is found through `OCCT_INCLUDE_DIR` and `OCCT_LIB_DIR`, falling back to common system
//! paths. OCCT 7.8 merged the data-exchange toolkits (TKSTEP, TKIGES, ...) into TKDESTEP,
//! TKDEIGES and friends; the version header decides which names to link.

use std::path::{Path, PathBuf};

const INCLUDE_CANDIDATES: &[&str] = &[
    "/usr/include/opencascade",
    "/usr/local/include/opencascade",
    "/opt/homebrew/include/opencascade",
    "/usr/local/opt/opencascade/include/opencascade",
];

const LIB_CANDIDATES: &[&str] = &[
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/usr/lib64",
    "/usr/local/lib",
    "/opt/homebrew/lib",
    "/usr/local/opt/opencascade/lib",
    "/usr/lib",
];

/// Toolkits needed by every build, in dependency order (users first).
const COMMON_LIBS: &[&str] = &[
    "TKOffset",
    "TKMesh",
    "TKBO",
    "TKPrim",
    "TKShHealing",
    "TKTopAlgo",
    "TKGeomAlgo",
    "TKBRep",
    "TKGeomBase",
    "TKG3d",
    "TKG2d",
    "TKMath",
    "TKernel",
];

/// Data-exchange toolkits from OCCT 7.8 on.
const DE_LIBS_NEW: &[&str] = &["TKDESTEP", "TKDEIGES", "TKDESTL", "TKDE", "TKXSBase"];

/// Data-exchange toolkits before OCCT 7.8.
const DE_LIBS_OLD: &[&str] = &[
    "TKSTEP",
    "TKSTEP209",
    "TKSTEPAttr",
    "TKSTEPBase",
    "TKIGES",
    "TKSTL",
    "TKXSBase",
];

fn main() -> Result<(), String> {
    for var in ["OCCT_INCLUDE_DIR", "OCCT_LIB_DIR"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=shim/flatbello_occt.h");
    println!("cargo:rerun-if-changed=shim/flatbello_occt.cpp");

    let include = locate("OCCT_INCLUDE_DIR", INCLUDE_CANDIDATES, |d| {
        d.join("Standard_Version.hxx").is_file()
    })
    .ok_or(
        "OCCT headers not found: set OCCT_INCLUDE_DIR to the folder with Standard_Version.hxx",
    )?;
    let lib_dir = locate("OCCT_LIB_DIR", LIB_CANDIDATES, has_tkernel)
        .ok_or("OCCT libraries not found: set OCCT_LIB_DIR to the folder with TKernel")?;
    let (major, minor) = occt_version(&include.join("Standard_Version.hxx"))?;

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("shim/flatbello_occt.cpp")
        .include("shim")
        .include(&include)
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter");
    if std::env::var_os("CARGO_FEATURE_FIXTURES").is_some() {
        build.define("FB_FIXTURES", None);
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.flag("/EHsc").flag("/bigobj");
    }
    build.compile("flatbello_occt_shim");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    let (preferred, other) = if (major, minor) >= (7, 8) {
        (DE_LIBS_NEW, DE_LIBS_OLD)
    } else {
        (DE_LIBS_OLD, DE_LIBS_NEW)
    };
    // Link the toolkits of the detected naming scheme that exist, plus any of the other
    // scheme's that exist too, so a later rename does not silently drop a library.
    let mut linked = Vec::new();
    for lib in preferred.iter().chain(other).chain(COMMON_LIBS) {
        if !linked.contains(lib) && library_exists(&lib_dir, lib) {
            println!("cargo:rustc-link-lib=dylib={lib}");
            linked.push(*lib);
        }
    }
    let missing: Vec<&str> = COMMON_LIBS
        .iter()
        .chain(
            preferred
                .iter()
                .filter(|l| **l != "TKDESTL" && **l != "TKSTL"),
        )
        .filter(|l| !linked.contains(l))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "OCCT {major}.{minor} in {} lacks {}",
            lib_dir.display(),
            missing.join(", ")
        ));
    }
    Ok(())
}

/// The directory named by `var`, or the first candidate accepted by `ok`.
fn locate(var: &str, candidates: &[&str], ok: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(var) {
        return Some(PathBuf::from(dir));
    }
    candidates.iter().map(PathBuf::from).find(|d| ok(d))
}

fn library_exists(dir: &Path, name: &str) -> bool {
    [
        format!("lib{name}.so"),
        format!("lib{name}.dylib"),
        format!("{name}.lib"),
    ]
    .iter()
    .any(|f| dir.join(f).exists())
}

fn has_tkernel(dir: &Path) -> bool {
    ["libTKernel.so", "libTKernel.dylib", "TKernel.lib"]
        .iter()
        .any(|f| dir.join(f).exists())
}

/// Reads OCC_VERSION_MAJOR and OCC_VERSION_MINOR from Standard_Version.hxx.
fn occt_version(header: &Path) -> Result<(u32, u32), String> {
    let text = std::fs::read_to_string(header)
        .map_err(|e| format!("reading {}: {e}", header.display()))?;
    let field = |name: &str| {
        text.lines()
            .find_map(|l| {
                let mut words = l.split_whitespace();
                (words.next() == Some("#define") && words.next() == Some(name))
                    .then(|| words.next()?.parse::<u32>().ok())
                    .flatten()
            })
            .ok_or(format!("{name} not found in {}", header.display()))
    };
    Ok((field("OCC_VERSION_MAJOR")?, field("OCC_VERSION_MINOR")?))
}
