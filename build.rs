use std::env;
use std::path::PathBuf;

fn main() {
    let aidl_dir = PathBuf::from("aidl");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let panel_iface =
        aidl_dir.join("vendor/oplus/hardware/displaypanelfeature/IDisplayPanelFeature.aidl");

    println!("cargo:rerun-if-changed={}", panel_iface.display());
    assert!(
        panel_iface.exists(),
        "missing AIDL source: {}",
        panel_iface.display()
    );

    // Hashes are the frozen-AIDL hashes shipped in the stock ROM binaries:
    // vendor.oplus.hardware.displaypanelfeature-V1-ndk.so and
    // android.hardware.light-V1-ndk_platform.so.
    let panel_out = out_dir.join("panel_bindings.rs");
    rsbinder_aidl::Builder::new()
        .source(panel_iface)
        .include_dir(&aidl_dir)
        .version(1)
        .hash("39afca22ac77253421ad681eeb2bae0c6ff62c13")
        .output(&panel_out)
        .generate()
        .expect("failed to generate IDisplayPanelFeature bindings");
}
