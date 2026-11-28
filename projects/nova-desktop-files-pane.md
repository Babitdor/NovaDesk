
## Files pane (2026-11, after 0.9.0) — tabs, preview kinds, fork
- `FilesPane` owns the tab strip; `PreviewPanel` owns navigation, the unsaved dialog and the read-only lock. A tab's `✕` renders only on *background* tabs — the active one closes through the preview header, which is where the guard lives. Leaving a tab always settles its draft, so background tabs are always clean.
- Tool-driven opens (`open={path,nonce}` from `nova:open-file`) land read-only; the lock is decided as `read && current === path` so it is known on the first render. Tree opens are editable.
- Preview kinds now include `pdf`/`video`/`audio`, served by the loopback preview server (data URLs cannot answer Range). `preview.rs` answers 206 with `Accept-Ranges: bytes`; `media-src` in tauri.conf CSP must include `http://127.0.0.1:*` or the player is blocked.
- `MAX_READ_BYTES` is 2 MiB (was 256 KiB, a hard error). `search_files` (Ctrl+P) is a bounded, skip-listed stack walk: depth 12, 20k entries, 100 results, skips .git/node_modules/target/dist/...
- Ctrl+P is captured on the Files pane's own root, not window-wide, because every pane stays mounted while hidden.
- Fork (`forkChat` in sessions.ts) copies the transcript through the forked item and *keeps* `session`, so the branch inherits the harness conversation; pinned flag and teamRun are not copied.

## Packaged Files fixes (0.10.1)
- CodeMirror-generated styles must receive the existing Tauri style nonce via `EditorView.cspNonce`. `style-src` includes a nonce in packaged builds, so `unsafe-inline` does not authorize an unmarked generated stylesheet. Test the embedded app, not just Vite.
- Reopening a tab from the tree must create a request even when its path matches `shown`: Markdown self-navigation may have left `request.path` pointing at the original file.
- `node scripts/verify-tools.mjs --packaged-files` requires an embedded frontend and checks both regressions plus save, conflict and draft behavior.
