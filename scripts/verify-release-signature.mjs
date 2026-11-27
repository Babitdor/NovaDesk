import { readFile, readdir } from "node:fs/promises"
import { createHash, createPublicKey, verify as verifySignature } from "node:crypto"
import { join } from "node:path"

// Checks published installers against the key this build trusts, which is the
// check the updater performs. Before 0.8.0, one corrupted byte in the shipped
// public key made correctly signed installers fail verification. The release
// workflow now runs this check before publishing.
//
//   gh release download v0.7.0 --pattern "*.msi*" --pattern "*.exe*" --dir out
//   node scripts/verify-release-signature.mjs out
//
// Exit code 1 when an artifact and its .sig disagree, which is what a release
// job should treat as a failure.

/// A minisign file is base64 of "untrusted comment" + base64 of the payload; the
/// updater hands us the first layer, then decodes the payload itself.
function inner(base64Text) {
  const text = Buffer.from(base64Text.trim(), "base64").toString("utf8").trim().split(/\r?\n/)
  return { comment: text[0], blob: Buffer.from(text[1] ?? "", "base64") }
}

function trustedKey(pubkey) {
  const { blob } = inner(pubkey)
  // sig_alg (2) + keynum (8) + key (32); Node wants an SPKI wrapper for Ed25519.
  const spki = Buffer.concat([
    Buffer.from([0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00]),
    blob.subarray(10),
  ])
  return createPublicKey({ key: spki, format: "der", type: "spki" })
}

/// `ED` is the prehashed mode: the signature covers BLAKE2b-512 of the file.
function check(bytes, sigText, key) {
  const { blob } = inner(sigText)
  const prehashed = blob.subarray(0, 2).toString("latin1") === "ED"
  const message = prehashed ? createHash("blake2b512").update(bytes).digest() : bytes
  return { prehashed, ok: verifySignature(null, message, key, blob.subarray(10)) }
}

const folder = process.argv[2]
if (!folder) {
  console.error("usage: node scripts/verify-release-signature.mjs <folder with installers and .sig files>")
  process.exit(2)
}
const { plugins } = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"))
const key = trustedKey(plugins.updater.pubkey)

const files = await readdir(folder)
const signatures = files.filter((name) => name.endsWith(".sig"))
if (signatures.length === 0) {
  console.error("no .sig files in " + folder)
  process.exit(2)
}
let failed = 0
for (const name of signatures) {
  const artifact = join(folder, name.slice(0, -4))
  try {
    const result = check(await readFile(artifact), await readFile(join(folder, name), "utf8"), key)
    console.log((result.ok ? "ok   " : "FAIL ") + name + (result.prehashed ? "" : " (legacy mode)"))
    if (!result.ok) failed += 1
  } catch (error) {
    console.log("FAIL " + name + ": " + error.message)
    failed += 1
  }
}
process.exit(failed === 0 ? 0 : 1)
