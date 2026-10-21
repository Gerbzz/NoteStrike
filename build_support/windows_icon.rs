//! Dependency-free Windows icon resource support. This is build-time branding only.
use std::{env, fs, path::PathBuf, process::Command};

fn sdk_rc() -> Option<PathBuf> {
    let host = match env::var("PROCESSOR_ARCHITECTURE").as_deref() {
        Ok("ARM64") => "arm64",
        Ok("x86") => "x86",
        _ => "x64",
    };
    if let (Some(root), Some(version)) = (
        env::var_os("WindowsSdkDir"),
        env::var_os("WindowsSDKVersion"),
    ) {
        let rc = PathBuf::from(root)
            .join("bin")
            .join(version)
            .join(host)
            .join("rc.exe");
        if rc.is_file() {
            return Some(rc);
        }
    }
    let root = PathBuf::from(env::var_os("ProgramFiles(x86)")?).join("Windows Kits/10/bin");
    let mut versions: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("10."))
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|v| v.join(host).join("rc.exe"))
        .find(|p| p.is_file())
}

pub fn embed(icon_relative: &str, binary: &str) {
    println!("cargo:rerun-if-changed={icon_relative}");
    println!("cargo:rerun-if-changed=build_support/windows_icon.rs");
    for name in [
        "RC",
        "WindowsSdkDir",
        "WindowsSDKVersion",
        "STRIKE_REQUIRE_ICON",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join(icon_relative);
    assert!(icon.is_file(), "missing app icon: {}", icon.display());
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let source = output.join("strike-icon.rc");
    let quoted = icon
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"");
    fs::write(&source, format!("1 ICON \"{quoted}\"\n")).expect("write icon resource source");
    let gnu = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    let compiled = output.join(if gnu {
        "strike-icon.o"
    } else {
        "strike-icon.res"
    });
    let compiler = env::var_os("RC").map(PathBuf::from).or_else(|| {
        if gnu {
            Some(PathBuf::from("windres"))
        } else {
            sdk_rc()
        }
    });
    let outcome = compiler
        .ok_or_else(|| "Windows SDK resource compiler not found".to_string())
        .and_then(|compiler| {
            let mut command = Command::new(compiler);
            if gnu {
                command
                    .arg("-i")
                    .arg(&source)
                    .arg("-O")
                    .arg("coff")
                    .arg("-o")
                    .arg(&compiled);
            } else {
                command
                    .arg("/nologo")
                    .arg("/fo")
                    .arg(&compiled)
                    .arg(&source);
            }
            command
                .output()
                .map_err(|e| e.to_string())
                .and_then(|result| {
                    if result.status.success() && compiled.is_file() {
                        Ok(())
                    } else {
                        Err(format!(
                            "resource compiler failed: {}",
                            String::from_utf8_lossy(&result.stderr)
                        ))
                    }
                })
        });
    match outcome {
        Ok(()) => println!("cargo:rustc-link-arg-bin={binary}={}", compiled.display()),
        Err(error) => {
            if env::var_os("STRIKE_REQUIRE_ICON").is_some() {
                panic!("Windows app icon resource: {error}");
            }
            println!(
                "cargo:warning=Windows app icon resource was not embedded: {error}; set RC to the resource compiler path, or STRIKE_REQUIRE_ICON=1 to require it"
            );
        }
    }
}
