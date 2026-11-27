import { spawn } from 'node:child_process'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { setTimeout as wait } from 'node:timers/promises'

// Measures what a launch costs, in the two halves that are actually separate:
// the window (process spawn -> page target -> React painted its first screen)
// and the agent behind it (`nova --mode pipe` booting to its ready frame).
//
//   npm run build && npm run tauri build      # the release binary it measures
//   node scripts/measure-launch.mjs [runs]
//
// It runs the shipped binary against a throwaway WebView2 profile, seeds a
// trusted workspace on the first launch, then times the launches a returning
// user gets. Nothing here touches the real profile: APPDATA, LOCALAPPDATA,
// USERPROFILE and WEBVIEW2_USER_DATA_FOLDER all point into a temp directory.

const exe = process.env.NOVA_EXE ?? 'src-tauri/target/release/novacode-desktop.exe'
const runs = Number(process.argv[2] ?? 3)

function freePort() {
  return new Promise((done) => {
    const server = createServer()
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port
      server.close(() => done(port))
    })
  })
}

/// One Runtime.evaluate over a fresh CDP socket; the webview does not keep one
/// open for us between polls.
async function evaluate(socketUrl, expression) {
  const socket = new WebSocket(socketUrl)
  try {
    const { result, exceptionDetails } = await new Promise((done, fail) => {
      const signal = AbortSignal.timeout(8000)
      signal.addEventListener('abort', () => fail(signal.reason), { once: true })
      socket.addEventListener('error', () => fail(new Error('webview closed')), { once: true })
      socket.addEventListener('open', () => socket.send(JSON.stringify({
        id: 1, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true },
      })), { once: true })
      socket.addEventListener('message', ({ data }) => {
        const reply = JSON.parse(data)
        if (reply.id !== 1) return
        if (reply.error) fail(new Error(reply.error.message))
        else done(reply.result)
      })
    })
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text)
    return result.value
  } finally { socket.close() }
}

const PROBE = 'JSON.stringify((() => {' +
  'const nav = performance.getEntriesByType("navigation")[0];' +
  'return { domContentLoaded: nav ? Math.round(nav.domContentLoadedEventEnd) : null,' +
  ' loadEvent: nav ? Math.round(nav.loadEventEnd) : null,' +
  ' curtainGone: !document.getElementById("boot"),' +
  ' jsBytes: performance.getEntriesByType("resource")' +
  '   .filter((e) => e.name.endsWith(".js"))' +
  '   .reduce((n, e) => n + (e.decodedBodySize || 0), 0) };' +
  '})())'

async function launch(home, workspace, { seed = false } = {}) {
  const port = await freePort()
  const started = performance.now()
  const app = spawn(resolve(exe), [], {
    windowsHide: true,
    env: { ...process.env,
      APPDATA: join(home, 'appdata'),
      LOCALAPPDATA: join(home, 'localappdata'),
      USERPROFILE: join(home, 'profile'),
      WEBVIEW2_USER_DATA_FOLDER: join(home, 'webview'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=' + port },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  const log = []
  const record = (chunk) => {
    for (const line of String(chunk).split(/\r?\n/)) {
      if (line.trim()) log.push(Math.round(performance.now() - started) + 'ms ' + line.trim())
    }
  }
  app.stdout.on('data', record)
  app.stderr.on('data', record)
  const marks = {}
  try {
    let socket, targetAt
    for (let i = 0; i < 800; i++) {
      try {
        const targets = await (await fetch('http://127.0.0.1:' + port + '/json/list')).json()
        const page = targets.find(({ type }) => type === 'page')
        if (page?.webSocketDebuggerUrl) { socket = page.webSocketDebuggerUrl; targetAt = performance.now() - started; break }
      } catch {}
      await wait(10)
    }
    if (!socket) throw new Error('no page target within 10 s')
    marks.spawnToPageTarget = Math.round(targetAt)
    for (let i = 0; i < 900; i++) {
      const painted = await evaluate(socket, 'document.getElementById("root")?.childElementCount ?? 0').catch(() => 0)
      if (painted > 0) { marks.spawnToFirstDom = Math.round(performance.now() - started); break }
      await wait(10)
    }
    if (seed) {
      const info = await evaluate(socket, 'window.__TAURI_INTERNALS__.invoke("set_workspace", ' + JSON.stringify({ path: workspace }) + ')')
      await evaluate(socket, 'localStorage.setItem("nova.desktop.workspace", ' + JSON.stringify(info.root) + ');' +
        'localStorage.setItem("nova.desktop.settings.v1", ' +
        JSON.stringify(JSON.stringify({ trustedWorkspaces: [info.root], widgetEnabled: false, quickLauncher: false })) + ');' +
        '"seeded"')
      marks.seeded = info.root
    } else {
      await wait(400)
      marks.page = JSON.parse(await evaluate(socket, PROBE).catch(() => '{}'))
    }
    marks.log = log.slice(0, 8)
  } finally {
    app.kill()
    await wait(600)
  }
  return marks
}

/// The agent half: `nova --mode pipe` to its ready frame, which is what the app
/// pays once per workspace before a turn can stream.
async function agentBoot(cwd) {
  const { createInterface } = await import('node:readline')
  const started = performance.now()
  const child = spawn('nova', ['--mode', 'pipe'], { cwd, stdio: ['pipe', 'pipe', 'pipe'] })
  const lines = createInterface({ input: child.stdout })
  let ready = null
  await new Promise((done) => {
    lines.on('line', (line) => {
      try { if (JSON.parse(line).type === 'ready') { ready = Math.round(performance.now() - started); done() } } catch {}
    })
    child.on('error', done)
    child.on('exit', done)
    setTimeout(done, 60000)
  })
  child.kill()
  return ready
}

const home = await mkdtemp(join(tmpdir(), 'nova-launch-'))
const workspace = join(home, 'workspace')
const results = []
try {
  for (const dir of ['appdata', 'localappdata', 'profile', 'webview', 'workspace/src']) {
    await mkdir(join(home, dir), { recursive: true })
  }
  for (let i = 0; i < 40; i++) {
    await writeFile(join(workspace, 'src', 'file-' + i + '.ts'), 'export const n' + i + ' = ' + i + '\n')
  }
  await launch(home, workspace, { seed: true })
  for (let i = 0; i < runs; i++) {
    const marks = await launch(home, workspace)
    results.push(marks)
    console.log('launch ' + (i + 1) + ': ' + JSON.stringify(marks))
    await wait(400)
  }
} finally {
  await rm(home, { recursive: true, force: true }).catch(() => {})
}

const median = (values) => {
  const sorted = values.filter((v) => typeof v === 'number').sort((a, b) => a - b)
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null
}

console.log('median spawn -> page target: ' + median(results.map((r) => r.spawnToPageTarget)) + ' ms')
console.log('median spawn -> first paint: ' + median(results.map((r) => r.spawnToFirstDom)) + ' ms')
const boot = await agentBoot(process.cwd())
console.log('agent boot (nova --mode pipe -> ready): ' + (boot === null ? 'no ready frame' : boot + ' ms'))