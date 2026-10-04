// v1.1.0.3 - Embed one multi-resolution Windows icon without an external resource compiler.
#[path = "src/windows_icon_resource.rs"]
mod windows_icon_resource;

fn main() {
    println!("cargo:rerun-if-changed=assets/windows/tenrate.ico");
    println!("cargo:rerun-if-changed=src/windows_icon_resource.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let bytes = std::fs::read(root.join("assets/windows/tenrate.ico"))
        .expect("the Windows application icon must exist");
    let resource = windows_icon_resource::icon_resource(&bytes)
        .expect("the Windows application icon must be a complete ICO file");
    let output =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("tenrate_icon.res");
    std::fs::write(&output, resource).expect("write Windows application icon resource");
    for binary in ["timer_windows_client", "tenrate_desktop_launcher"] {
        println!("cargo:rustc-link-arg-bin={binary}={}", output.display());
    }
}
