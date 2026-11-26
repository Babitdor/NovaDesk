// Generates src/data/notices.ts — the licence list the About sheet renders.
//
// Everything is read from this checkout, never the network: package-lock.json
// plus the installed package.json files for npm, Cargo.lock plus the registry
// copies already on this machine for crates. A crate whose source is not on
// disk is written as "unknown" and reported, so the file never invents a
// licence it could not see.
//
// Run with: npm run notices
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { homedir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { gunzipSync } from 'node:zlib'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const output = join(root, 'src', 'data', 'notices.ts')

const licenseField = (text) => {
  const match = /^\s*license\s*=\s*"([^"]+)"/m.exec(text)
  const table = /^\s*license-file\s*=\s*"([^"]+)"/m.exec(text)
  if (match) return match[1]
  if (table) return `see ${table[1]}`
  const list = /^\s*licenses\s*=\s*\[([^\]]+)\]/m.exec(text)
  return list ? list[1].replace(/[\s"]|type\s*=\s*/g, '').replace(/^,|,$/g, '') : null
}

// Only the [package] table counts; a dependency table can carry a licence key.
function cargoLicense(text) {
  const start = text.indexOf('[package]')
  if (start < 0) return licenseField(text) ?? 'unknown'
  const rest = text.slice(start + '[package]'.length)
  const end = rest.search(/^\[/m)
  return licenseField(end < 0 ? rest : rest.slice(0, end)) ?? 'unknown'
}

// Modern manifests carry a single `license`; older ones a `licenses` array.
function npmLicense(manifest) {
  if (typeof manifest.license === 'string' && manifest.license) return manifest.license
  if (manifest.license?.type) return manifest.license.type
  const list = Array.isArray(manifest.licenses)
    ? manifest.licenses.filter((entry) => entry?.type).map((entry) => entry.type)
    : []
  return list.length ? list.join(' OR ') : 'unknown'
}

function npmNotices() {
  const lock = JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8'))
  const notices = []
  for (const [key, entry] of Object.entries(lock.packages ?? {})) {
    // The root entry has an empty key; links point at workspace-local paths.
    if (!key.startsWith('node_modules/') || entry.link) continue
    if (entry.dev) continue
    const name = entry.name ?? key.slice(key.lastIndexOf('node_modules/') + 'node_modules/'.length)
    const manifest = join(root, key, 'package.json')
    const license = existsSync(manifest) ? npmLicense(JSON.parse(readFileSync(manifest, 'utf8'))) : 'unknown'
    notices.push({ name, version: entry.version ?? '0.0.0', license, kind: 'npm' })
  }
  return notices
}

function registryDirs(kind) {
  const dir = join(homedir(), '.cargo', 'registry', kind)
  return existsSync(dir) ? readdirSync(dir).map((name) => join(dir, name)) : []
}

// A .crate file is a gzipped tar whose manifest sits one directory deep, so a
// 512-byte header walk finds it and no tar dependency is needed.
function crateManifest(path) {
  const archive = gunzipSync(readFileSync(path))
  for (let offset = 0; offset + 512 <= archive.length;) {
    const header = archive.subarray(offset, offset + 512)
    const size = parseInt(header.subarray(124, 136).toString().replace(/[^0-7]/g, '') || '0', 8)
    const name = header.subarray(0, 100).toString().replace(/\0[\s\S]*$/, '')
    if (/^[^/]+\/Cargo\.toml$/.test(name)) {
      return archive.subarray(offset + 512, offset + 512 + size).toString('utf8')
    }
    offset += 512 + Math.ceil(size / 512) * 512
  }
  return null
}

function cargoNotices() {
  const text = readFileSync(join(root, 'src-tauri', 'Cargo.lock'), 'utf8')
  const sources = registryDirs('src')
  const caches = registryDirs('cache')
  const notices = []
  for (const block of text.split('[[package]]').slice(1)) {
    const name = /^\s*name = "([^"]+)"/m.exec(block)?.[1]
    const version = /^\s*version = "([^"]+)"/m.exec(block)?.[1]
    if (!name || !version) continue
    // Nova itself is not a third-party notice.
    if (name === 'novacode-desktop') continue
    // The patched ort-sys lives in the checkout, so it wins over the registry.
    const vendored = join(root, 'src-tauri', 'vendor', name, 'Cargo.toml')
    let license = 'unknown'
    if (existsSync(vendored)) license = cargoLicense(readFileSync(vendored, 'utf8'))
    if (license === 'unknown') {
      for (const dir of sources) {
        const manifest = join(dir, `${name}-${version}`, 'Cargo.toml')
        if (!existsSync(manifest)) continue
        license = cargoLicense(readFileSync(manifest, 'utf8'))
        break
      }
    }
    // Crates for other platforms were never extracted, but their .crate file
    // is usually still cached from the last download.
    if (license === 'unknown') {
      for (const dir of caches) {
        const archive = join(dir, `${name}-${version}.crate`)
        if (!existsSync(archive)) continue
        const manifest = crateManifest(archive)
        if (!manifest) continue
        license = cargoLicense(manifest)
        break
      }
    }
    notices.push({ name, version, license, kind: 'cargo' })
  }
  return notices
}

const quote = (value) => JSON.stringify(value)
const byKind = (a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version)
const entries = [...npmNotices(), ...cargoNotices()].sort(byKind)

const body = entries
  .map(({ name, version, license, kind }) =>
    `  { name: ${quote(name)}, version: ${quote(version)}, license: ${quote(license)}, kind: ${quote(kind)} },`)
  .join('\n')

const file = `// Generated by \`npm run notices\` — do not edit by hand.
//
// Sources: package-lock.json and the installed package.json files, plus
// src-tauri/Cargo.lock and the cargo registry copies on the machine that ran
// the script. A package whose manifest was missing is listed as "unknown"
// rather than guessed.
export type Notice = {
  name: string
  version: string
  license: string
  kind: 'npm' | 'cargo'
}

export const NOTICES: Notice[] = [
${body}
]
`

mkdirSync(dirname(output), { recursive: true })
writeFileSync(output, file)

const unknown = entries.filter(({ license }) => license === 'unknown')
const summary = new Map()
for (const { license } of entries) summary.set(license, (summary.get(license) ?? 0) + 1)
console.log(`notices: ${entries.length} entries (${entries.filter((entry) => entry.kind === 'npm').length} npm, ${entries.filter((entry) => entry.kind === 'cargo').length} cargo) -> src/data/notices.ts`)
for (const [license, count] of [...summary].sort((a, b) => b[1] - a[1])) console.log(`  ${String(count).padStart(4)}  ${license}`)
console.log(`  unknown: ${unknown.length}`)
if (unknown.length) console.log(`  no licence found for: ${unknown.map(({ name, version }) => `${name}@${version}`).join(', ')}`)
