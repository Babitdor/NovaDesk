
## About pane + deeper theming (verified 2026-11-26)
- `src-tauri/src/commands/app.rs`: `app_report` (version, identifier, os/arch, RSS, uptime,
  live turns, open terminals, app_data_dir) and `check_app_update` (GitHub releases/latest via the
  existing `commands::settings::download` curl helper + `semver`). RSS is a raw
  `K32GetProcessMemoryInfo` FFI struct; it matched `Get-Process … WorkingSet64` **exactly**
  (50,700,288 B), so the `cb` layout is right. A unit test asserts a plausible range.
- `scripts/notices.mjs` (`npm run notices`) writes `src/data/notices.ts` from package-lock +
  Cargo.lock. This machine: **497 entries, 131 "unknown"** — those crates (gtk/webkit/objc2/
  windows-0.42 …) are genuinely absent from `~/.cargo/registry` (the .crate cache holds the same
  756 crates as src, nothing extra). Do not "fix" the count upward by guessing.
- Theming lives in `src/lib/settings.ts`: a `TOKENS` table maps workbench keys → Nova vars, with
  `color-mix` derivations for anything the theme omits, so sidebar/panel/dock/scrollbars all follow.
  `PRESETS` (phosphor/midnight/slate/parchment/contrast) use real workbench keys so imported themes
  and presets share one path. Accent: `accentFromTheme` toggle + per-scheme `customAccent`/
  `customAccentLight` + `recentAccents`; `--accent-line` = accent mixed 55 %.
  `applyAppearance` takes the *theme's* mode when a theme is active, so the Appearance select must
  clear `themeId` (it already does) or switching light/dark appears to do nothing.
- The debug exe still needs `npm run dev`. CDP check: launch with
  `WEBVIEW2_USER_DATA_FOLDER=<temp>` **plus** `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=
  --remote-debugging-port=9333`; this time the temp profile really was isolated (fresh localStorage,
  the user's release instance untouched). Pick the window by
  `__TAURI_INTERNALS__.metadata.currentWebview.label === 'main'`.
  `document.body.innerText` is uppercased by `text-transform`, so a readiness check for visible copy
  must lowercase first.
- Verifying the offline branch: launch a second instance with `HTTPS_PROXY=http://127.0.0.1:9` —
  curl fails fast and the row renders "Could not reach GitHub: The command did not complete
  successfully" while `updateCheckedAt` stays 0.
- oxlint's react rules (0-warning bar) flag any `setState` reachable from an effect body, including
  `void refresh()`. Pattern that passes: wrap the sampling in `useEffectEvent`, then let the effect
  only `setTimeout(open, 0)` + `setInterval(tick, …)`.
- Driving React-owned state from CDP: seed `nova.desktop.workspace` in localStorage and reload before
  opening the dock, and start a turn with `await import('/src/lib/ipc.ts')` (vite serves the module)
  plus `backend: 'local'`; UUID chat/turn ids are required. Rust outlives a page reload but its
  terminal map does not reset — a reloaded page leaves orphan shells counted in "Open terminals", so
  restart the process for a clean 0 → 1.


---


## `cargo test --lib` hangs on a loader dialog (verified 2026-11)
Cargo puts the lib unit-test binary under
`src-tauri/target/debug/build/novacode-desktop/<hash>/out/novacode_lib-<hash>.exe`, and it
carries no manifest, so the loader binds ComCtl32 v5 and the process dies on the missing
`TaskDialogIndirect` import behind a modal "Entry Point Not Found" box that hangs the run.
`build.rs`'s `cargo:rustc-link-arg-tests` does not reach this target; the README's RUSTFLAGS
`/MANIFEST:EMBED` recipe is the supported route. Cheaper re-verification without rebuilding
every dependency: copy that exe, then
`mt.exe -manifest comctl6.manifest "-outputresource:copy.exe;#1"` (SDK x64 mt lives in
`C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\`), and run the copy — it
lists and runs the same tests.

## Update check and GitHub's anonymous quota (fixed 2026-11)
`check_app_update` reads `api.github.com/repos/Babitdor/NovaDesk/releases/latest` through the
system curl; anonymous calls are 60/h per address, a spent quota answers 403, and curl
`--fail` turned that into exit 22. That is why the pane printed "Could not reach GitHub: The
command did not complete successfully". `settings::download_response` now keeps the status
(`--write-out "\n%{http_code}"`, no `--fail`), and 403/429/5xx fall back to
`github.com/Babitdor/NovaDesk/releases/latest`, whose redirect names the tag with no quota.


---


## Driving the built app over WebView2 CDP (GUI probes)
Launch the debug exe with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=N`, a
throwaway `WEBVIEW2_USER_DATA_FOLDER`, and APPDATA/LOCALAPPDATA/USERPROFILE pointed into a temp dir;
then `fetch http://127.0.0.1:N/json/list` for the page target and speak `Runtime.evaluate`
(`awaitPromise`, `returnByValue`) on its websocket. Two traps cost real time (2026-11-28):

* **The boot curtain blocks hit-testing.** `#boot` in `index.html` is lifted by `main.tsx` only after
  two `requestAnimationFrame`s + `document.fonts.ready` (1.5 s timeout) + 380 ms. Any
  `location.reload()` — which a probe needs after writing the workspace into localStorage — puts it
  back for ~2 s. `elementFromPoint` then returns `DIV#boot`, so **real** CDP mouse clicks go nowhere
  while `.click()` from `Runtime.evaluate` still works: DOM-only assertions pass while the window is
  unusable. Wait for `!document.getElementById('boot')` after every reload.
* **A packaged-mode debug binary serves `http://tauri.localhost/`**, not the vite devUrl. `tauri build
  --debug --no-bundle` leaves that variant behind, so a probe that matches `http://localhost:1420/`
  finds no page target; match any `type === 'page'` target instead. A later plain `cargo build` does
  **not** restore the dev-mode binary (cargo sees it as up to date).

Typing into a CodeMirror editor needs a real click (`Input.dispatchMouseEvent`) for focus, then
`Input.insertText`; Ctrl+S is `Input.dispatchKeyEvent` `keyDown`/`keyUp` with `modifiers: 2`.
Judge layout with `getBoundingClientRect()` — reading `textContent` proves nothing about whether a
pane has width (a closed `.right.is-closed` column still renders its whole subtree at 0 px).
