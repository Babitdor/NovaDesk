# Runs the Rust test suite the way CI does, so no test binary ever dies in the
# Windows loader with STATUS_ENTRYPOINT_NOT_FOUND.
#
# Why the split: test code links rfd (via tauri-plugin-dialog and muda), which
# imports ComCtl32 v6's TaskDialogIndirect. Windows loads v5 for an executable
# whose manifest does not ask for v6, and the loader kills the process before
# the harness prints a line. Each target kind gets its manifest differently:
#   - the app binary: tauri-build embeds one (never add RUSTFLAGS here — a
#     second manifest fails to link with CVT1100 / LNK1123);
#   - integration tests: build.rs hands each one the v6 manifest;
#   - the lib's unit-test binary: cargo has no per-target link args for it, so
#     RUSTFLAGS supplies the manifest for this one invocation.

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\src-tauri')

# whisper-rs-sys rebuilds through CMake; a shell that does not have CMake on
# PATH (the VS BuildTools copy is not always exported) would fail halfway.
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
  $vsCmake = Get-ChildItem 'C:\Program Files (x86)\Microsoft Visual Studio' -Recurse -Filter 'cmake.exe' -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
  if ($vsCmake) {
    $env:PATH += ';' + (Split-Path $vsCmake)
    Write-Host "Using CMake from $vsCmake"
  }
}

$manifest = Join-Path $env:TEMP 'comctl6.manifest'
@'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
</assembly>
'@ | Set-Content -Path $manifest -Encoding utf8

cargo test --workspace --all-features --bins
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo test -p novacode-desktop --all-features `
  --test speak_engine --test speak_onnx --test speak_service `
  --test tools_mcp --test tools_shim --test voice_engine
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$env:RUSTFLAGS = "-C link-arg=/MANIFEST:EMBED -C link-arg=/MANIFESTINPUT:$manifest"
cargo test --workspace --all-features --lib
exit $LASTEXITCODE
