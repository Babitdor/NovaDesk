import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer as httpServer } from 'node:http'
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { setTimeout as wait } from 'node:timers/promises'
import { createServer } from 'vite'

// The prewarm contract: one Nova process per workspace when the folder opens,
// spent by the first turn, which boots its replacement for the next new chat,
// and then the chat reuses what it has. Turn two must neither wait for that
// replacement nor start a third process. Run after `cargo build`, with port
// 1420 free, like the tools smoke test.
assert.equal(process.platform, 'win32', 'This check needs Windows and WebView2')

const READY_DELAY = Number(process.env.READY_DELAY ?? 1500)

async function eventually(fn, label) {
  let cause
  for (let i = 0; i < 120; i++) {
    try {
      return await fn()
    } catch (error) {
      cause = error
    }
    await wait(150)
  }
  throw new Error(label, { cause })
}

async function evaluate(webSocketUrl, expression) {
  const socket = new WebSocket(webSocketUrl)
  try {
    const { result, exceptionDetails } = await new Promise((done, fail) => {
      const signal = AbortSignal.timeout(15000)
      signal.addEventListener('abort', () => fail(signal.reason), { once: true })
      socket.addEventListener('error', () => fail(new Error('WebView debugger disconnected')), { once: true })
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
  } finally {
    socket.close()
  }
}

const root = await mkdtemp(join(tmpdir(), 'nova-prewarm-'))
const workspace = join(root, 'workspace')
const spawnLog = join(root, 'spawns.log')
for (const dir of ['workspace/src', 'profile', 'appdata', 'localappdata']) {
  await mkdir(join(root, dir), { recursive: true })
}
await writeFile(join(workspace, 'src', 'fixture.ts'), 'export const fixture = 1\n')
await writeFile(spawnLog, '')

const fake = join(root, 'nova.cjs')
await writeFile(fake, [
  "const { appendFileSync } = require('node:fs')",
  "const { createInterface } = require('node:readline')",
  "appendFileSync(process.env.NOVA_SPAWN_LOG, Date.now() + '\\n')",
  "const emit = (frame) => console.log(JSON.stringify(frame))",
  "setTimeout(() => emit({ type: 'ready', model: 'fixture', capabilities: ['steer'] }), Number(process.env.NOVA_READY_DELAY))",
  "createInterface({ input: process.stdin }).on('line', (line) => {",
  "  const frame = JSON.parse(line)",
  "  if (frame.type === 'command') emit({ type: 'command_result', ok: true })",
  "  if (frame.type !== 'prompt') return",
  "  emit({ type: 'text', id: frame.id, content: 'fixture says ' + frame.content })",
  "  emit({ type: 'done', id: frame.id, status: 'completed', ok: true })",
  "})",
].join('\n'))
await writeFile(join(root, 'nova.cmd'), '@"' + process.execPath + '" "' + fake + '" %*\r\n')

const portProbe = httpServer().listen(0, '127.0.0.1')
await once(portProbe, 'listening')
const debugPort = portProbe.address().port
await new Promise((done) => portProbe.close(done))
const vite = await createServer({ server: { host: 'localhost', port: 1420, strictPort: true, hmr: false } })
let app
try {
  await vite.listen()
  app = spawn(resolve('src-tauri/target/debug/novacode-desktop.exe'), [], {
    windowsHide: true,
    env: {
      ...process.env,
      APPDATA: join(root, 'appdata'),
      LOCALAPPDATA: join(root, 'localappdata'),
      USERPROFILE: join(root, 'profile'),
      NOVA_BIN: join(root, 'nova.cmd'),
      NOVA_SPAWN_LOG: spawnLog,
      NOVA_READY_DELAY: String(READY_DELAY),
      WEBVIEW2_USER_DATA_FOLDER: join(root, 'webview'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=' + debugPort,
    },
    stdio: ['ignore', 'ignore', 'inherit'],
  })

  const webview = await eventually(async () => {
    assert.ok(app.exitCode === null, 'desktop process exited')
    const targets = await (await fetch('http://127.0.0.1:' + debugPort + '/json/list')).json()
    const target = targets.find(({ url }) => url.startsWith('http://localhost:1420/'))
    assert.ok(target?.webSocketDebuggerUrl)
    return target.webSocketDebuggerUrl
  }, 'No main window')
  const pageEval = (source) => evaluate(webview, source)
  const waitFor = (source, label) => eventually(async () => assert.ok(await pageEval(source)), label)

  await waitFor('Boolean(document.querySelector(".rail-settings"))', 'Desktop settings did not render')
  const info = await pageEval('window.__TAURI_INTERNALS__.invoke("set_workspace", ' + JSON.stringify({ path: workspace }) + ')')
  const settings = JSON.stringify({ trustedWorkspaces: [info.root], widgetEnabled: false, quickLauncher: false })
  await pageEval(
    'localStorage.setItem("nova.desktop.workspace", ' + JSON.stringify(info.root) + ');' +
    'localStorage.setItem("nova.desktop.settings.v1", ' + JSON.stringify(settings) + ');' +
    'setTimeout(() => location.reload(), 50); "seeded"',
  )
  await wait(1500)

  const boots = async () => (await readFile(spawnLog, 'utf8')).split('\n').filter(Boolean).length
  await eventually(async () => assert.ok((await boots()) >= 1), 'Workspace never prewarmed Nova')
  await eventually(async () => {
    const loaded = await pageEval('(async () => { try { window.__ipc = await import("/src/lib/ipc.ts") } catch (e) { return "ERR " + e.message } return Object.keys(window.__ipc).join(",") })()')
    assert.ok(String(loaded).includes('startTurn'), 'App IPC module did not load: ' + loaded)
  }, 'App IPC module did not load')

  const turn = (chatId, prompt) => pageEval(
    '(async () => {' +
    ' const started = performance.now();' +
    ' window.__events = []; window.__finished = null; window.__latency = null;' +
    ' await window.__ipc.startTurn({ turnId: crypto.randomUUID(), chatId: ' + JSON.stringify(chatId) +
    ', prompt: ' + JSON.stringify(prompt) + ', backend: "nova", trustWorkspace: true }, (event) => {' +
    ' if (window.__latency === null && event.type === "messageFinal") window.__latency = Math.round(performance.now() - started);' +
    ' window.__events.push(event.type);' +
    ' if (event.type === "turnFinished" || event.type === "turnError") window.__finished = event;' +
    ' });' +
    ' return "started" })()',
  )
  const settle = async (label) => {
    await eventually(async () => assert.equal(await pageEval('Boolean(window.__finished)'), true), label + ' never finished')
    const finished = await pageEval('JSON.stringify(window.__finished)')
    assert.ok(finished.includes('"reason":"completed"'), label + ' did not complete: ' + finished)
    return pageEval('window.__latency')
  }

  const chat = crypto.randomUUID()
  await turn(chat, 'first')
  const first = await settle('First turn')
  console.log('first turn events: ' + await pageEval('window.__events.join(",")'))
  await eventually(async () => assert.equal(await boots(), 2), 'First turn did not leave a replacement prewarm')
  console.log('first turn: first text after ' + first + ' ms, ' + (await boots()) + ' Nova process(es)')

  await turn(chat, 'second')
  const second = await settle('Second turn')
  // The turn's own replacement, if it wrongly starts one, spawns after the
  // reply; give it time to write its line before counting.
  await wait(600)
  const after = await boots()
  assert.equal(after, 2, 'Second turn booted a third Nova; the replacement ran more than once')
  assert.ok(second < 500, 'Second turn waited ' + second + ' ms for the replacement prewarm')
  console.log('second turn: first text after ' + second + ' ms, still ' + after + ' Nova process(es)')
  console.log('prewarm check passed')
} finally {
  app?.kill()
  await vite.close()
  await wait(400)
  await rm(root, { recursive: true, force: true }).catch(() => {})
}
