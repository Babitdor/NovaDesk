// Copying the tool shim where Tauri bundles it from.
//
// Tauri's externalBin rule is fixed: the file has to exist as
// src-tauri/binaries/nova-tools-<target-triple>.exe before a bundle is built,
// and it lands beside the app executable renamed to nova-tools.exe. The app
// resolves it there at runtime (src-tauri/src/tools/mod.rs, shim()).
//
// Run by npm run tools:shim, which CI and the release workflow call before
// building a bundle. A plain development build needs nothing: cargo puts both
// binaries in the same target directory, which is the other place shim() looks.
import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const crate = resolve(here, '../src-tauri')
const release = !process.argv.includes('--debug')
const profile = release ? 'release' : 'debug'

// The shim is its own workspace member, so this builds without touching the
// app crate — which could not build anyway until the file below exists.
execFileSync('cargo', ['build', '-p', 'nova-tools', ...(release ? ['--release'] : [])], {
  cwd: crate,
  stdio: 'inherit',
})

const version = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
const triple = version.split('\n').find((line) => line.startsWith('host: '))?.slice(6).trim()
if (!triple) {
  console.error('could not read the host triple from rustc -vV')
  process.exit(1)
}

const extension = process.platform === 'win32' ? '.exe' : ''
const built = join(crate, 'target', profile, 'nova-tools' + extension)
if (!existsSync(built)) {
  console.error('cargo built no nova-tools at ' + built)
  process.exit(1)
}
const destination = join(crate, 'binaries', 'nova-tools-' + triple + extension)
mkdirSync(dirname(destination), { recursive: true })
copyFileSync(built, destination)
console.log('shim copied to ' + destination)
