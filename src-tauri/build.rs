/// ComCtl32 v6 for the test targets.
///
/// rfd's message dialogs — reached through tauri-plugin-dialog, which the app
/// registers — import `TaskDialogIndirect`, and that export exists only in
/// ComCtl32 v6. Windows loads `System32\comctl32.dll`, which is v5, for an
/// executable whose manifest does not ask for v6, and the process then dies
/// inside the loader with STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) before the
/// test harness prints a single line.
///
/// The app binary is covered: tauri-build embeds a manifest for it. Integration
/// test binaries are separate link steps with nothing, so this adds one for the
/// test targets only. The lib's own unit-test binary cannot be reached from
/// here — cargo has no per-target flag for it — and gets the same manifest from
/// RUSTFLAGS instead; see the Checks section of the README.
///
/// The file lives in the temp folder because link.exe cannot take a
/// /MANIFESTINPUT path containing spaces, and this repository is checked out
/// under "B:\Summer Project 2026\...".
const COMCTL6: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        let manifest = std::env::temp_dir().join("nova-comctl6.manifest");
        if std::fs::write(&manifest, COMCTL6).is_err() {
            println!("cargo:warning=could not write the ComCtl32 v6 manifest for test targets");
        }
        // -tests only: the app binary already carries tauri-build's manifest and a
        // second embedded one there fails to link (LNK1123).
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
    tauri_build::build()
}
