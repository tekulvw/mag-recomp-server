use std::{env, fs, path::PathBuf, process::Command};
fn main() {
    let root = PathBuf::from("vendor/recastnavigation");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut sources = vec![PathBuf::from("src/navmesh_bridge.cpp")];
    for dir in ["Recast", "Detour"] {
        let mut entries: Vec<_> = fs::read_dir(root.join(dir).join("Source"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "cpp"))
            .collect();
        entries.sort();
        sources.extend(entries);
    }
    let compiler = env::var("CXX").unwrap_or_else(|_| "c++".into());
    let mut objects = Vec::new();
    for (i, source) in sources.iter().enumerate() {
        let object = out.join(format!("navmesh_{i}.o"));
        assert!(
            Command::new(&compiler)
                .args([
                    "-std=c++11",
                    "-O2",
                    "-fPIC",
                    "-fno-exceptions",
                    "-fno-rtti",
                    "-Ivendor/recastnavigation/Recast/Include",
                    "-Ivendor/recastnavigation/Detour/Include",
                    "-c"
                ])
                .arg(source)
                .arg("-o")
                .arg(&object)
                .status()
                .unwrap()
                .success(),
            "C++ navmesh compilation failed"
        );
        objects.push(object);
    }
    let archive = out.join("libmag_navmesh.a");
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(&archive)
            .args(&objects)
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=mag_navmesh");
    println!(
        "cargo:rustc-link-lib={}",
        if env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos" {
            "c++"
        } else {
            "stdc++"
        }
    );
    println!("cargo:rerun-if-changed=src/navmesh_bridge.cpp");
    println!("cargo:rerun-if-changed=vendor/recastnavigation");
}
