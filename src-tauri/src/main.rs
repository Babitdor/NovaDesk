// Keeps a console window from flashing open behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(debug_assertions))]
const _: () = assert!(
    !tauri::is_dev(),
    "Build the Windows release with npm run tauri -- build --no-bundle to embed the frontend."
);

fn main() {
    novacode_lib::run()
}

#[cfg(all(test, feature = "custom-protocol"))]
#[test]
fn packaged_desktop_embeds_its_frontend() {
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    assert!(!tauri::is_dev());
    assert!(context.assets().get(&"index.html".into()).is_some());
}
