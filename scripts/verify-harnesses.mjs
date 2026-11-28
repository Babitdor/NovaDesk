import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { copyFile, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { basename, join, resolve, sep } from 'node:path'
import { createServer } from 'vite'
import { createServer as httpServer } from 'node:http'
import { setTimeout as wait } from 'node:timers/promises'

assert.equal(process.platform, 'win32')
const root = await mkdtemp(join(tmpdir(), 'nova-harnesses-'))
// A concurrent development build must not replace or lock the running fixture.
const debug = resolve('src-tauri/target/debug')
for (const name of ['novacode-desktop.exe', 'nova-tools.exe', ...(await readdir(debug)).filter((name) => name.endsWith('.dll'))]) {
  await copyFile(join(debug, name), join(root, name === 'novacode-desktop.exe' ? 'desktop-probe.exe' : name))
}

const workspace = join(root, 'workspace')
await mkdir(workspace)
const probe = httpServer().listen(0, '127.0.0.1')
await once(probe, 'listening')
const port = probe.address().port
await new Promise((resolve) => probe.close(resolve))
const vite = await createServer({ cacheDir:join(root, 'vite-cache'), server:{ host:'localhost', port:1420, strictPort:true, hmr:false, watch:null } })
let app
let diagnostics = ''
const awareness = process.argv.includes('--awareness')
const resultPath = awareness ? '.temp/harness-awareness-results.json' : '.temp/harness-results.json'
const results = JSON.parse(await readFile(resultPath, 'utf8').catch((error) => { if (error.code === 'ENOENT') return '[]'; throw error }))
assert.ok(Array.isArray(results))
const readOnly = process.argv.includes('--read-only')
async function eventually(fn, timeout = 20_000) {
  const start = Date.now()
  let last
  while (Date.now() - start < timeout) {
    try { return await fn() } catch (error) { last = error }
    await wait(150)
  }
  throw last ?? new Error('Timed out')
}
async function evaluate(url, expression) {
  const socket = new WebSocket(url)
  try {
    return await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('WebView evaluation timed out: ' + expression.slice(0, 140))), 60000)
      socket.addEventListener('error', reject, { once:true })
      socket.addEventListener('open', () => socket.send(JSON.stringify({ id:1, method:'Runtime.evaluate', params:{ expression, awaitPromise:true, returnByValue:true } })), { once:true })
      socket.addEventListener('message', ({ data }) => {
        const reply = JSON.parse(data)
        if (reply.id !== 1) return
        clearTimeout(timeout)
        if (reply.error || reply.result.exceptionDetails) reject(new Error(JSON.stringify(reply.error ?? reply.result.exceptionDetails)))
        else resolve(reply.result.result.value)
      })
    })
  } finally { socket.close() }
}
try {
  await vite.listen()
  app = spawn(join(root, 'desktop-probe.exe'), [], { windowsHide:true,
    env:{ ...process.env, APPDATA:join(root, 'appdata'),
      WEBVIEW2_USER_DATA_FOLDER:join(root, 'webview'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=' + port },
    stdio:['ignore','ignore','pipe'] })
  app.stderr.on('data', (bytes) => { diagnostics = (diagnostics + bytes).slice(-12000) })
  const debuggerUrl = await eventually(async () => {
    assert.equal(app.exitCode, null, diagnostics)
    const targets = await (await fetch('http://127.0.0.1:' + port + '/json/list')).json()
    const main = targets.find((target) => target.url.startsWith('http://localhost:1420/'))
    assert.ok(main?.webSocketDebuggerUrl)
    return main.webSocketDebuggerUrl
  })
  const page = (expression) => evaluate(debuggerUrl, expression)
  await eventually(async () => assert.ok(await page('Boolean(window.__TAURI_INTERNALS__)')))
  const info = await page(`window.__TAURI_INTERNALS__.invoke('set_workspace', { path:${JSON.stringify(workspace)} })`)
  await page(`localStorage.setItem('nova.desktop.workspace', ${JSON.stringify(info.root)});
    localStorage.setItem('nova.desktop.settings.v1', ${JSON.stringify(JSON.stringify({ trustedWorkspaces:[info.root], widgetEnabled:false, quickLauncher:false, speak:false }))});
    setTimeout(() => location.reload(), 50)`)
  await wait(1000)
  console.log('Workspace ready; detecting installed harnesses')
  const statuses = await page(`window.__TAURI_INTERNALS__.invoke('agent_status', { versions:true })`)
  await page(`window.__TAURI_INTERNALS__.invoke('set_auto_approve', { enabled:true })`)
  const selected = process.argv.slice(2).filter((argument) => !argument.startsWith('--'))
  for (const harness of statuses.filter((status) => status.installed && (!readOnly || status.id === 'codex') && (!selected.length || selected.includes(status.id)))) {
    console.log('Verifying ' + harness.id)
    const row = { harness:harness.id, version:harness.version, runs:[] }
    let chatId = crypto.randomUUID()
    const title = 'Harness probe ' + harness.id
    const prompt = readOnly ? 'Verify read-only enforcement by attempting a direct write of the text proof to read-only-proof.txt using your command execution tool. Do not request escalation. Report the actual denial. Do not use MCP tools.' : awareness ? `Create a task in this app titled "${title}", then change its title to "Verified ${title}". Do this in the app's visible task list, not a private checklist. Reply briefly when done.` : `Use the create_task tool from the nova-desktop MCP server to create a task titled "${title}". Then use rename_task from that same MCP server to change that task title to "Verified ${title}". The MCP taskId must be a UUID. Use these MCP tools for Desktop’s visible task list. Do not use the shell or edit files. Reply with one short sentence after the two tool calls succeed.`
    for (const mode of readOnly ? ['read-only'] : awareness ? ['cold', 'warm'] : ['cold', 'warm', 'cold', 'warm']) {
      if (mode === 'cold') chatId = crypto.randomUUID()
      // Exercise both ACP injection and precedence of a saved local registration.
      if (harness.id === 'commandcode' && mode === 'cold' && row.runs.length) {
        await page(`window.__TAURI_INTERNALS__.invoke('register_commandcode_tools')`)
      }
      if (awareness && harness.id === 'nova' && mode === 'cold') {
        await page(`window.__TAURI_INTERNALS__.invoke('prewarm_nova', { backend:'nova', chatId:${JSON.stringify(chatId)} })`)
        await wait(150)
      }
      const turnId = crypto.randomUUID()
      const previous = mode === 'warm' ? row.runs.at(-1)?.session : undefined
      const request = { turnId, chatId, backend:harness.id, prompt, trustWorkspace:true, session:previous, readOnly }
      if (readOnly) {
        const rejection = await page(`(async () => {
          const { startTurn } = await import('/src/lib/ipc.ts');
          try { await startTurn(${JSON.stringify({ ...request, check:['cmd.exe','/c','echo bypass > read-only-proof.txt'] })}, () => {}); return 'unexpected success' }
          catch (error) { return error.message }
        })()`)
        assert.match(rejection, /Read-only investigation cannot run a project check/)
      }
      await page(`(async () => {
        const { startTurn } = await import('/src/lib/ipc.ts');
        window.__probeEvents = []; window.__probeStarted = performance.now();
        await startTurn(${JSON.stringify(request)}, (event) => window.__probeEvents.push({ ...event, elapsedMs:Math.round(performance.now() - window.__probeStarted) }));
      })()`)
      let events
      try {
        events = await eventually(async () => {
          const events = await page('window.__probeEvents')
          assert.ok(events.some((event) => event.type === 'turnFinished'))
          return events
        }, 180_000)
      } catch {
        await page(`window.__TAURI_INTERNALS__.invoke('cancel_turn', { turnId:${JSON.stringify(turnId)} })`)
        events = await page('window.__probeEvents')
      }
      const catalog = await page(`window.__TAURI_INTERNALS__.invoke('tools_catalog')`)
      const calls = catalog.activity.filter((call) => call.turnId === turnId)
      const chats = await page(`JSON.parse(localStorage.getItem('nova.chats.v1') ?? '[]')`)
      const visible = chats.some((chat) => chat.title === 'Verified ' + title)
      row.runs.push({ mode, session:events.findLast((event) => event.type === 'session')?.id,
        firstTextMs:events.find((event) => ['messageDelta','messagePreview','messageFinal','finalAnswerDelta'].includes(event.type))?.elapsedMs,
        finish:events.findLast((event) => event.type === 'turnFinished')?.reason ?? 'timeout',
        calls, observableResult:visible, readOnly, writeExists:readOnly ? await readFile(join(workspace, 'read-only-proof.txt')).then(() => true, () => false) : undefined, discovered:catalog.listedTools.includes(harness.id),
        tools:events.filter((event) => event.type === 'toolCall' || event.type === 'toolResult'),
        reply:events.filter((event) => event.type === 'messageFinal').map((event) => event.text).join('\n'),
        timings:events.filter((event) => event.type === 'timing'),
        decisions:events.filter((event) => event.type === 'approvalRequired' || event.type === 'approvalResolved'),
        notices:events.filter((event) => event.type === 'notice' || event.type === 'turnError').map((event) => event.message) })
      console.log(JSON.stringify({ harness:row.harness, ...row.runs.at(-1) }))
      if (readOnly) {
        const proof = row.runs.at(-1)
        assert.equal(proof.finish, 'completed')
        assert.equal(proof.writeExists, false)
        assert.ok(proof.tools.some((event) => event.type === 'toolResult' && event.ok === false && /denied|read.only/i.test(event.output)), 'No actual denied write was observed')
      }
      if (row.runs.at(-1).finish !== 'completed') break
    }
    row.capabilities = (await page(`window.__TAURI_INTERNALS__.invoke('agent_capabilities')`))[harness.id]
    const previous = results.findIndex((entry) => entry.harness === harness.id && Boolean(entry.readOnly) === readOnly)
    row.readOnly = readOnly
    if (previous >= 0) results[previous] = row
    else results.push(row)
    await mkdir('.temp', { recursive:true })
    await writeFile(resultPath, JSON.stringify(results, null, 2) + '\n')
    if (awareness) {
      assert.equal(row.runs.length, 2, harness.id + ' did not complete both session checks')
      for (const run of row.runs) {
        assert.equal(run.finish, 'completed', harness.id + ' turn failed')
        for (const name of ['create_task', 'rename_task']) {
          assert.ok(run.calls.some((call) => call.name === name && call.ok), harness.id + ' did not choose ' + name + ' for the ordinary app request')
        }
        assert.equal(run.observableResult, true)
      }
    }
  }
} catch (error) {
  console.error('Desktop probe failure; process exit:', app?.exitCode, 'stderr:', diagnostics)
  throw error
} finally {
  if (app && app.exitCode === null) { app.kill(); await Promise.race([once(app, 'exit').catch(() => {}), wait(5000)]) }
  await vite.close()
  assert.ok(resolve(root).startsWith(resolve(tmpdir()) + sep) && basename(root).startsWith('nova-harnesses-'))
  await rm(root, { recursive:true, force:true, maxRetries:6, retryDelay:250 }).catch((error) => console.error('Temporary workspace cleanup:', error.code))
}
