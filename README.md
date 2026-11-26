<div align="center">

<img src="screenshots/home.png" alt="Nova Desktop — first run" width="100%">

# Nova Desktop

**A desktop shell for coding agents.** Open a folder, pick a harness, and work the code
next to the transcript: diffs, files, a browser and real shells in one window.

Tauri 2 + React 19 · Windows-first · one workspace, many agents

[![CI](https://github.com/Babitdor/NovaDesk/actions/workflows/ci.yml/badge.svg)](https://github.com/Babitdor/NovaDesk/actions/workflows/ci.yml)
[![Release](https://github.com/Babitdor/NovaDesk/actions/workflows/release.yml/badge.svg)](https://github.com/Babitdor/NovaDesk/actions/workflows/release.yml)
[![Latest release](https://img.shields.io/github/v/release/Babitdor/NovaDesk?label=latest&color=b891f7)](https://github.com/Babitdor/NovaDesk/releases/latest)

</div>

---

## What it is

Nova Desktop is not another chat window. It is the cockpit around a coding agent:
the agent runs as a child process bound to one workspace folder, and everything it
touches — the files it reads, the commands it runs, the diff it leaves behind — is one
keystroke away from the conversation that caused it.

- **Pick a harness.** Nova (the default, `nova --mode pipe`), Claude Code, Codex,
  OpenCode, Pi Agent, CommandCode and Hermes are all first-class; each is detected,
  versioned and updated from the app. A `local` offline stub is available for demos
  and tests.
- **One turn, one transcript.** Tool calls, streamed output and file edits fold into
  the same message stream, so history and screen can never disagree.
- **The crew.** Address a specialist from the composer — `@nora` researches,
  `@serena` makes surgical edits, `@ralph` grinds through retry loops, and `@team`
  runs the whole handoff (Nova → Nora → Serena → Ralph → Nova), each reply folded
  into the next prompt.
- **Change review in the loop.** The right panel reads the working tree with plain
  `git diff` / `git status` through the same process runner the terminal uses, and
  groups the result per file with its own line counts.
- **Real shells.** A dock of ConPTY-backed sessions (PowerShell, Command Prompt, Git
  Bash) rooted in the workspace, up to eight at a time.
- **Browser pane.** A native child webview for checking the thing you just built,
  plus a list of local dev servers to jump to.
- **A sky that answers the hand.** The empty plate is a canvas starfield: three depth
  layers that lean with the pointer, the stars nearest the cursor brighten, a click
  throws a burst of sparks, and meteors cross every few seconds. `Still sky` in
  Appearance freezes it to one frame, and reduced-motion does the same on its own.
- **Slash commands and models.** `/model`, `/new`, `/stop`, `/terminal`, `/theme`,
  `/mcp`, `/auth` and the full Nova command set autocomplete in the composer.
- **Settings that persist.** Appearance (dark/light/system), nine accents, imported
  VS Code colour themes, per-role agent teams and model routing, project checks after
  a successful turn, schedules, read-aloud and MCP connections.

## Screenshots

| A task, its transcript and the working-tree diff |
| --- |
| <img src="screenshots/task.png" alt="Task transcript with the Changes panel open"> |

| Terminal dock | Appearance | Agents |
| --- | --- | --- |
| <img src="screenshots/terminal.png" alt="PowerShell dock under the composer"> | <img src="screenshots/settings-appearance.png" alt="Appearance settings: theme, wallpaper, accent, VS Code themes"> | <img src="screenshots/settings-agents.png" alt="Agents settings: installed harnesses and versions"> |

## Requirements

- Windows 10/11 (WebView2 is bundled on 11; 10 needs the runtime installed).
- **Node 20.19+ or 22.12+** — Vite 8 does not run on older releases.
- **Rust stable** (MSVC toolchain) with the Tauri 2 prerequisites.
- **Python 3** on `PATH` — the terminal dock spawns a small helper script for its
  ConPTY sessions.
- The agent CLIs you intend to use, on `PATH` and authenticated (`claude auth login`,
  `codex login`, `nova init`, …). The app reports what is missing.

## Run it

```powershell
git clone https://github.com/Babitdor/NovaDesk.git
cd NovaDesk
npm install
npm run tauri dev      # desktop window with hot reload
```

Frontend only, in a browser tab (IPC calls are unavailable there, so the app renders
but cannot drive a workspace):

```powershell
npm run dev            # http://localhost:1420
```

## Voice

Voice turns speech into text on this machine. The microphone is captured in the
webview, the samples are resampled to 16 kHz mono in Rust, and a local ONNX model
transcribes them; nothing is uploaded, and the recording is discarded once the text
is in the composer. Speech is transcribed by [`transcribe-rs`](https://github.com/cjpais/transcribe-rs)
on top of ONNX Runtime.

Turn it on in **Settings → Voice**: download a model, set *Send microphone audio to*
to **On-device**, then press `Ctrl+Shift+Space` or the **Dictate** button in the
composer. The shortcut can toggle or record only while held. The first time the
microphone is opened, WebView2 asks whether Nova may use it — that dialog is the
system's, and nothing records until it is allowed.

The catalogue lists 52 models with the copy, error rates, decode speeds, sizes and
licences the vendors publish. Nine can run on this build:

| Model | Languages | Download | Licence |
| --- | --- | --- | --- |
| Parakeet TDT 0.6B v2 | English | 661 MB | CC-BY-4.0 |
| Parakeet TDT 0.6B v3 | 25 European languages | 671 MB | CC-BY-4.0 |
| Canary 180M Flash | English, German, Spanish, French | 213 MB | CC-BY-4.0 |
| Canary 1B Flash | English, German, Spanish, French | 939 MB | CC-BY-4.0 |
| Canary 1B v2 | 25 European languages | 1.03 GB | CC-BY-4.0 |
| Cohere Transcribe (int4) | 14 languages incl. Chinese, Japanese, Korean | 1.3 GB | Apache-2.0 |
| SenseVoice Small | Mandarin, English, Japanese, Korean, Cantonese | 160 MB | FunASR Model License 1.1 |
| Moonshine Base | English | 58 MB | MIT |
| Moonshine Tiny | English | 32 MB | MIT |

The other cards are listed, not hidden, and say why they cannot run yet: Whisper needs
the whisper.cpp engine, which this build does not link; the streaming exports (Moonshine
Streaming, Nemotron Speech Streaming, Parakeet Multitalker, Voxtral Realtime) assume a
live audio feed and Nova transcribes each recording after you stop; the remaining
families (Qwen3-ASR, Voxtral, Granite Speech, Fun-ASR, MOSS, Breeze, Canary-Qwen, Cohere
Arabic, the Parakeet CTC, RNN-T and 1.1B variants, Orukeet, Primeline, Parakeet Ultra)
have no loader in Nova. Each of those cards shows *Not supported yet* instead of a
download button, with the reason in its tooltip.

Sizes on the cards and in this table are the bytes the download fetches, not the
publisher's rounded figure, because the progress bar and the post-download size check
read the same number. Grades and speeds are unchanged from the vendors' benchmark.

Every installable model is fetched on demand — none ships inside the installer — and lands in
`%APPDATA%\app.novacode.desktop\voice\<model>\` (a `.nova-model.json` marker is
written last, so a half-finished download never looks installed). Deleting a model
from the same page removes the folder.

Keep an eye on the disk: Parakeet is the everyday choice at 671 MB, and Cohere holds
about 1.3 GB and needs several GB of free memory while a model is loaded. Only one
model is resident at a time, and it is dropped after five idle minutes.

Canary's own ONNX export ships no `nemo128.onnx` (the 128-bin NeMo feature
extractor), so it is fetched from the Parakeet export of the same upstream pipeline;
the catalogue in `src-tauri/src/voice/catalog.rs` is the one place those URLs live.

The accuracy grades and decode speeds on the cards come from the
[Handy](https://github.com/cjpais/Handy) models benchmark; sizes are the published
file sizes of the download a card offers.

ONNX Runtime is linked into the executable at build time (ort's prebuilt binaries are
static), so the app ships no separate runtime library to lose. Settings → Voice asks
ort to start its environment when the page opens and reports it if that fails, rather
than letting the first transcription die with a stack trace.

To check inference without the UI, run the ignored integration test against a
downloaded model and any 16 kHz mono WAV:

```powershell
$env:NOVA_VOICE_MODEL="C:\path\to\model-dir"   # the four extracted files
$env:NOVA_VOICE_WAV="C:\path\to\speech.wav"
cargo test --test voice_engine --manifest-path src-tauri/Cargo.toml -- --ignored
```

## Build installers

```powershell
npm run tauri build
```

Artifacts land in `src-tauri/target/release/`:

- `novacode-desktop.exe` — portable binary
- `bundle/msi/Nova Desktop_0.1.0_x64_en-US.msi`
- `bundle/nsis/Nova Desktop_0.1.0_x64-setup.exe`

Close a running copy of the app before rebuilding: Windows keeps the release exe
locked and the copy step fails with `Access is denied. (os error 5)`.

## Releases and tags

Releases are cut from tags, and the installers are built by CI rather than by hand.
The tag is the version: bump `version` in `src-tauri/tauri.conf.json` first, then tag
the commit you want to ship — `v0.1.0` for version `0.1.0`.

```powershell
git tag -a v0.1.0 -m "Nova Desktop 0.1.0"
git push origin v0.1.0
```

`.github/workflows/release.yml` then runs on `windows-latest`, builds the MSI and NSIS
bundles with `tauri-apps/tauri-action`, and publishes a GitHub release named after the
tag with both installers attached:

| Asset | What it is |
| --- | --- |
| `Nova.Desktop_<version>_x64-setup.exe` | NSIS installer (per-user, no admin) |
| `Nova.Desktop_<version>_x64_en-US.msi` | MSI package for managed installs |

GitHub replaces the spaces in the bundle names with dots, so the attached assets read
`Nova.Desktop_…` while the local files under `src-tauri/target/release/bundle/` keep
their spaces.

The workflow can also be started by hand (`workflow_dispatch`) from the Actions tab.
`concurrency` keeps one release build at a time — a second tag waits instead of
cancelling the first, so no run is left half-published.

Tags are plain `v<version>` — `git tag -l` lists them, and `gh release list` shows what
shipped with each one. Nothing in the app depends on a release existing; the portable
`novacode-desktop.exe` from a local build is the same binary.

## General, Schedules and agent requests

The **General** pane is where the window and the process are configured, and every
control in it is wired to something real:

| Control | What it changes |
| --- | --- |
| Interface size | CSS `zoom` on the document (`preferences.interfaceSize`, 50-200). Ctrl +, Ctrl - and Ctrl 0 do the same. |
| Compact conversation / Expand tool activity | Transcript density and whether tool details open by default. |
| Global quick launcher | A chord registered with the OS (`tauri-plugin-global-shortcut`) that opens a 560x96 frameless window. It hands one prompt to the main window through `nova:quick-task` and has no commands of its own. |
| Running agents widget | A frameless companion window fed by `nova:widget-state`; clicking a row emits `nova:focus-chat`. Pinning keeps it above other applications. |
| Restart after a crash / Crash reports | Written to `recovery.json` in the app data folder, where the panic hook reads it. Reports land in `crashes/crash-<ms>.json`; the restart budget is three per fifteen minutes. |
| Keep unsent drafts | Composer text in `nova.drafts.v1`, 30 days and a 200k-character cap. |
| Projects | The folders you opened, newest first. Remove forgets the entry; the folder and its workspace trust are untouched. |

**Schedules** gained four shapes besides "once at a date": daily (all days or
weekdays only), every N minutes, after the running task finishes, and a chain of
prompts. The engine is `src/lib/scheduler.ts` - pure functions, run with
`node --test` - so "a due run fires once" is a property of the data instead of a
side effect of the 5-second ticker. Old `{ at, daily }` entries migrate on load.
Pause all suppresses new runs; a run already going is left alone.

### The agent request file

An agent cannot call a command: the invoke guard admits the main window only, and
there is no CLI flag to invent. The contract is a file instead.

```text
NOVA_DESKTOP_REQUESTS=<app data>/requests/<task or personality>.jsonl
```

The variable is set on every harness process Nova Desktop spawns. The file is cleared
before a turn and read after it - at most 20 lines and 64 KiB of strict JSON, one
object per line - and each valid line becomes one event:

```json
{"type":"wakeup","prompt":"re-run the failing tests","agent":"nova","afterSeconds":60}
{"type":"threadMessage","thread":"<task id>","text":"the schema changed"}
```

Anything malformed, too long or of an unknown type is dropped, and a harness that
ignores the variable behaves exactly as before. Both requests pass two gates in
**Schedules**: *Let agents schedule wake-ups* (off by default) and *Ask before agents
message other threads* (on by default, which shows an approval card in the task the
request came from). *Agent limit per thread* (1-100, default 10) counts running
agent-originated work plus the queue. Every refusal is written to the Schedules
activity log rather than silently dropped.

What this is not: it cannot restart the process after an OS kill or a driver fault,
the widget cannot start or stop a turn, and a schedule only runs while the window is
open and the same folder is active.
## Checks

```powershell
npm run lint                          # oxlint
npm run build                         # tsc -b && vite build
node --test src/lib/*.test.ts         # scheduler engine and preferences migration
cargo test                            # inside src-tauri
```

CI runs exactly those on every push and pull request: the frontend job on Ubuntu, the
Rust job on Windows (the backend shells out to ConPTY and PowerShell, so the tests run on
the platform they target). Formatting is a gate — `cargo fmt --all` before you push.

Releases are built and attached automatically: bump `version` in
`src-tauri/tauri.conf.json`, then push a matching tag.

```powershell
git tag -a v0.1.0 -m "Nova Desktop 0.1.0"
git push origin v0.1.0
```

That runs `.github/workflows/release.yml`, which publishes the MSI and NSIS installers as
a GitHub release for that tag — see [Releases and tags](#releases-and-tags).

## Layout

```text
src/                 React shell — App.tsx, components/, lib/ (ipc, sessions, crew, settings)
src-tauri/src/       Rust backend — agent/ (Nova pipe + CLI harnesses), commands/, workspace.rs
src-tauri/src/commands/terminal.py   ConPTY helper spawned for each terminal session
src/assets/harness/ Vendor-published agent marks (see SOURCES.md for provenance)
scripts/             Maintenance: sync the model catalogue and slash-command list
screenshots/         Images referenced by this README
```

The frontend never touches the filesystem, the shell or the network directly: every
action is a Tauri command, and `WorkspaceGuard` canonicalises and confines every path
to the open folder before it is used.

## Third-party marks

The agent marks in the app are the ones each vendor publishes — a favicon, brand file or
repository symbol — kept byte-for-byte and never recoloured. `src/assets/harness/SOURCES.md`
lists the exact source URL for every file:

| Mark | Harness | From |
| --- | --- | --- |
| Claude spark | Claude Code | `claude.ai` favicon |
| OpenAI blossom | Codex | `openai.com` favicon |
| OpenCode logo | OpenCode | OpenCode brand page (transparent, dark) |
| CommandCode symbol | CommandCode | CommandCode brand page symbol |
| Pi pixel mark | Pi Agent | `pi-mono` app favicon |
| Hermes icon | Hermes | `hermes-agent` app icon |
| Nova spark | Nova Desktop | drawn for this project |

Each mark is a trademark of its owner and appears here only to identify the tool it
belongs to. Nova Desktop is not affiliated with, endorsed by or sponsored by these
projects.

## How it is put together

- **One backend trait, many harnesses.** `AgentBackend` is a single async trait; the
  Nova backend speaks newline-delimited JSON to a long-lived `nova --mode pipe` child,
  and the CLI harnesses adapt their own streaming formats onto the same events.
- **Channels, not polling.** Each turn owns a `tauri::ipc::Channel`, so deltas are
  delivered to the chat that started the turn even if you are looking elsewhere.
- **Minimum capability surface.** The main window is granted `core:app`, `core:event`
  and `core:path` only — no `fs`, `shell`, `http` or `dialog` permissions — and the CSP
  allows nothing but IPC and localhost.
- **Closing the app closes the work.** Window close and exit cancel every live turn,
  one-off command and terminal, so nothing is orphaned.
