import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer as httpServer } from 'node:http'
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { createInterface } from 'node:readline'
import { setTimeout as wait } from 'node:timers/promises'
import { createServer } from 'vite'

// Run after building the Windows debug app and shim. Use a temporary workspace,
// WebView profile and Nova pipe process so the test needs no model credentials.
assert.equal(process.platform, 'win32', 'This smoke test needs Windows and WebView2')

async function eventually(fn, label) {
  let cause
  for (let i = 0; i < 100; i++) {
    try { return await fn() } catch (error) { cause = error }
    await wait(150)
  }
  throw new Error(label, { cause })
}

async function evaluate(webSocketUrl, expression, method = 'Runtime.evaluate', params = { expression, awaitPromise:true, returnByValue:true }) {
  const socket = new WebSocket(webSocketUrl)
  try {
    const { result, exceptionDetails } = await new Promise((resolve, reject) => {
      const signal = AbortSignal.timeout(5000)
      signal.addEventListener('abort', () => reject(signal.reason), { once: true })
      socket.addEventListener('error', () => reject(new Error('WebView debugger disconnected')), { once: true })
      socket.addEventListener('open', () => socket.send(JSON.stringify({
        id: 1, method, params,
      })), { once: true })
      socket.addEventListener('message', ({ data }) => {
        const reply = JSON.parse(data)
        if (reply.id !== 1) return
        if (reply.error) reject(new Error(reply.error.message))
        else resolve(reply.result)
      })
    })
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text)
    return result?.value
  } finally {
    socket.close()
  }
}

const root = await mkdtemp(join(tmpdir(), 'nova-tools-smoke-'))
const workspace = join(root, 'workspace')
const board = join(workspace, '.nova', 'moodboards', 'fixture')
const skill = join(workspace, '.nova', 'skills', 'fixture')
await mkdir(board, { recursive: true })
await mkdir(skill, { recursive: true })
await mkdir(join(root, 'profile'))
await mkdir(join(root, 'appdata'))
await mkdir(join(root, 'localappdata'))
await writeFile(join(workspace, 'fixture.txt'), 'desktop fixture needle\n')
await writeFile(join(board, 'board.json'), '{"name":"Fixture board"}')
const pixel = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aK9sAAAAASUVORK5CYII='
await writeFile(join(board, 'one.png'), Buffer.from(pixel, 'base64'))
await writeFile(join(skill, 'SKILL.md'), '---\nname: fixture\ndescription: A local fixture\n---\n')

const fake = join(root, 'nova.cjs')
await writeFile(fake, `
const { createInterface } = require('node:readline')
const emit = (frame) => console.log(JSON.stringify(frame))
let pending = null
emit({ type: 'ready', model: 'fixture', capabilities: ['steer'] })
createInterface({ input: process.stdin }).on('line', (line) => {
  const frame = JSON.parse(line)
  if (frame.type === 'command') emit({ type: 'command_result', ok: true })
  if (frame.type === 'steer') {
    emit({ type: 'steer_result', id: frame.id, request_id: frame.request_id, ok: true })
    emit({ type: 'text', request_id: frame.request_id, content: 'fixture steered ' + frame.content })
  }
  if (pending && frame.type === 'cancel') {
    emit({ type:'done', request_id:pending.id, status:'cancelled', ok:false }); pending = null; return
  }
  if (pending && (frame.type === 'approval' || frame.type === 'interrupt_reply')) {
    emit({ type:'text', request_id:pending.id, content:'permission delivered' });
    emit({ type:'done', request_id:pending.id, status:'completed', ok:true }); pending = null; return
  }
  if (frame.type !== 'prompt') return
  frame.content = frame.content.split('\\n\\nDesktop session:')[0]
  if (frame.content === 'fixture-permission' || frame.content === 'fixture-question') {
    pending = frame
    emit({ type:'approval_required', request_id:frame.id, interrupt_id:'same-id',
      kind:frame.content === 'fixture-question' ? 'question' : 'tool', payload:{ question:'Fixture question?', tool:'fixture' } }); return
  }
  const finish = () => {
    emit({ type: 'text', id: frame.id, content: 'fixture response ' + frame.content })
    emit({ type: 'done', id: frame.id, status: 'completed', ok: true })
  }
  if (frame.content === 'hold') {
    emit({ type: 'text', id: frame.id, content: 'fixture turn running' })
    setTimeout(finish, 12000)
  } else finish()
})
`)
await writeFile(join(root, 'nova.cmd'), `@"${process.execPath}" "${fake}" %*\r\n`)

const site = httpServer((req, res) => {
  res.setHeader('content-type', 'text/html')
  res.end(`<title>Fixture ${req.url}</title>
    <main><div id="status">ready</div>
    <button id="insert" onclick="this.after(Object.assign(document.createElement('button'), {textContent:'Inserted'}))">Insert</button>
    <button id="original" onclick="document.querySelector('#status').textContent='clicked original'">Original</button>
    <button onclick="document.querySelector('#original').remove()">Remove</button>
    <input aria-label="Field" oninput="document.querySelector('#status').textContent='typed '+JSON.stringify(this.value)">
    <input type="password" value="fixture-password">
    <div contenteditable aria-label="Editor" oninput="document.querySelector('#status').textContent='edited '+this.textContent">initial</div>
    <a href="/second">Next page</a></main>`)
})
site.listen(0, '127.0.0.1')
await once(site, 'listening')
const port = site.address().port
const url = 'http://127.0.0.1:' + port + '/'
const portProbe = httpServer().listen(0, '127.0.0.1')
await once(portProbe, 'listening')
const debugPort = portProbe.address().port
await new Promise((resolve) => portProbe.close(resolve))
const vite = await createServer({ server: { host: 'localhost', port: 1420, strictPort: true, hmr: false } })
let app, shim, motionSession
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
      WEBVIEW2_USER_DATA_FOLDER: join(root, 'webview'),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: '--remote-debugging-port=' + debugPort,
    },
    stdio: ['ignore', 'inherit', 'inherit'],
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
  await pageEval(`localStorage.setItem('nova.desktop.workspace', ${JSON.stringify(info.root)});
    localStorage.setItem('nova.desktop.settings.v1', ${JSON.stringify(JSON.stringify({
      trustedWorkspaces: [info.root], widgetEnabled: false, quickLauncher: false,
    }))});
    setTimeout(() => location.reload(), 50);`)
  await wait(200)
  await waitFor('Boolean(document.querySelector(".rail-settings"))', 'Page did not reload')
  await wait(500)
  const previousConfig = await readFile(join(workspace, '.nova', 'mcp.json'), 'utf8').catch(() => '')
  await pageEval('window.__TAURI_INTERNALS__.invoke("prewarm_nova")')
  const config = await eventually(async () => {
    const text = await readFile(join(workspace, '.nova', 'mcp.json'), 'utf8')
    assert.notEqual(text, previousConfig, 'Legacy prewarm has not replaced the scoped entry yet')
    return JSON.parse(text)
  }, 'No managed MCP configuration')
  const entry = config.mcpServers['nova-desktop']
  shim = spawn(entry.command, entry.args, { windowsHide: true, env: { ...process.env, ...entry.env }, stdio: ['pipe', 'pipe', 'inherit'] })
  const output = createInterface({ input: shim.stdout })[Symbol.asyncIterator]()
  let id = 0
  async function rpc(method, params) {
    shim.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: ++id, method, params }) + '\n')
    const { value } = await output.next()
    assert.ok(value, 'shim closed')
    const response = JSON.parse(value)
    assert.equal(response.id, id)
    assert.ok(!response.error, JSON.stringify(response.error))
    return response.result
  }
  const tested = new Set()
  async function call(name, args = {}, error) {
    const reply = await rpc('tools/call', { name, arguments: args })
    const result = JSON.parse(reply.content[0].text)
    if (error) {
      assert.equal(reply.isError, true, name)
      assert.match(result, error)
    } else assert.equal(reply.isError, false, name + ': ' + JSON.stringify(result))
    tested.add(name)
    return result
  }
  assert.equal((await rpc('initialize', {})).serverInfo.name, 'nova-desktop')
  const { tools } = await rpc('tools/list', {})
  for (const name of ['team_delegate', 'team_follow_up', 'team_message', 'team_status']) await call(name, {}, /authenticated active teammate/)

  const catalog = await pageEval('window.__TAURI_INTERNALS__.invoke("tools_catalog")')
  assert.ok(catalog.listedTools.includes('nova'), 'The catalog did not record the shim connection')
  assert.ok((await call('list_projects')).projects.some((project) => project.path === info.root))
  assert.ok((await call('list_local_servers')).servers.some((listener) => listener.endsWith(':' + port)))
  assert.equal((await call('code_search', { pattern: 'fixture needle' })).matches[0].path, 'fixture.txt')
  assert.ok((await call('list_skills')).skills.some((item) => item.name === 'fixture'))
  assert.equal((await call('list_moodboards')).moodboards[0].id, 'fixture')
  assert.equal((await call('read_moodboard', { id: 'fixture' })).board.name, 'Fixture board')
  assert.equal((await call('read_moodboard_image', { id: 'fixture', image: 'one.png' })).base64, pixel)
  await call('read_moodboard_image', { id: 'fixture', image: 'board.json' }, /image type/)

  const { taskId } = await call('create_task', { title: 'Desktop fixture' })
  await call('rename_task', { taskId, title: 'Renamed fixture' })
  await call('pin_task', { taskId, pinned: true })
  await call('set_task_stage', { taskId, column: 'doing' })
  await call('open_task', { taskId: 'does-not-exist' }, /no task/)
  assert.equal((await call('open_task', { taskId })).opened, taskId)
  const taskList = await call('list_tasks')
  assert.equal(taskList.activeId, taskId)
  assert.ok(taskList.tasks.some((task) => task.id === taskId && task.pinned))
  assert.equal((await call('search_tasks', { query: 'Renamed fixture' })).hits[0].taskId, taskId)
  await call('start_task', { taskId, prompt: 'hold' })
  await eventually(async () => assert.ok((await call('list_subagents')).count > 0), 'No running turn')
  await eventually(async () => {
    const { messages } = await call('read_task', { taskId })
    assert.ok(messages.some((message) => message.text === 'fixture turn running'))
  }, 'The Nova pipe did not start the turn')
  const steering = await call('steer_task', { taskId, text: 'focus on the fixture' })
  assert.equal(steering.accepted, true)
  await eventually(async () => {
    const { messages, state } = await call('read_task', { taskId })
    assert.equal(state, 'running')
    assert.ok(messages.some((message) => message.text === 'fixture steered focus on the fixture'))
  }, 'Steering did not reach the running turn')
  assert.deepEqual((await call('list_queue', { taskId })).queue, [])
  const remove = await call('queue_message', { taskId, text: 'remove this' })
  assert.equal((await call('remove_queued', { id: remove.queued.id })).removed, true)
  await call('queue_message', { taskId, text: 'follow-up' })
  assert.equal((await call('list_queue', { taskId })).queue[0].text, 'follow-up')
  await call('interrupt_task', { taskId })
  await eventually(async () => {
    const { messages } = await call('read_task', { taskId })
    assert.ok(messages.some((message) => message.text === 'fixture response follow-up'))
  }, 'Queued follow-up not delivered')
  await call('send_message', { taskId, text: 'direct follow-up' })
  await eventually(async () => {
    const { messages } = await call('read_task', { taskId })
    assert.ok(messages.some((message) => message.text === 'fixture response direct follow-up'))
  }, 'Direct follow-up not delivered')
  await call('archive_task', { taskId })
  await call('open_file', { path: 'fixture.txt' })
  await waitFor('document.body.innerText.includes("desktop fixture needle")', 'File preview did not open')
  await call('open_terminal', { shell: 'cmd' })
  await waitFor(`Array.from(document.querySelectorAll('.dock-terminal-tab button')).some(
    (button) => button.textContent === 'Command Prompt 1' && button.getAttribute('aria-pressed') === 'true'
  )`, 'Command Prompt tab did not open')
  assert.equal(await pageEval('document.querySelectorAll(".dock-terminal-tab").length'), 1)
  await waitFor('Boolean(document.querySelector(".interactive-terminal:not([hidden]) .interactive-input:enabled"))', 'Command Prompt did not start')

  assert.equal((await call('open_browser', { url })).opened, url)
  let snapshot = await eventually(async () => {
    const value = await call('read_browser')
    assert.equal(value.title, 'Fixture /')
    return value
  }, 'Browser page did not load')
  assert.ok(!JSON.stringify(snapshot).includes('fixture-password'))
  let ref = (name) => snapshot.elements.find((element) => element.name === name).ref
  const original = ref('Original')
  await call('browser_click', { ref: ref('Insert') })
  await call('browser_click', { ref: original })
  const { path } = await call('capture_browser')
  assert.equal((await readFile(path)).subarray(0, 8).toString('hex'), '89504e470d0a1a0a')
  snapshot = await call('read_browser')
  assert.match(snapshot.text, /clicked original/)
  await call('browser_click', { ref: original }, /stale/)
  await call('browser_type', { ref: ref('Original'), text: 'bad target' }, /cannot accept/)
  await call('browser_type', { ref: ref('Field'), text: '  keep spaces  ' })
  await call('browser_type', { text: '' })
  assert.match((await call('read_browser')).text, /typed ""/)
  snapshot = await call('read_browser')
  await call('browser_type', { ref: ref('Editor'), text: 'editor works' })
  snapshot = await call('read_browser')
  assert.match(snapshot.text, /edited editor works/)
  const stale = ref('Original')
  await call('browser_click', { ref: ref('Next page') })
  await eventually(async () => assert.equal((await call('read_browser')).title, 'Fixture /second'), 'page navigation')
  await call('browser_click', { ref: stale }, /stale/)
  await call('browser_action', { action: 'back' })
  await eventually(async () => assert.equal((await call('read_browser')).title, 'Fixture /'), 'history back')
  await call('browser_action', { action: 'forward' })
  await eventually(async () => assert.equal((await call('read_browser')).title, 'Fixture /second'), 'history forward')
  await call('browser_action', { action: 'reload' })
  assert.deepEqual([...tested].sort(), tools.map(({ name }) => name).sort())
  // The backend owns permissions, cancellation and team writer exclusion.
  const teamId = crypto.randomUUID()
  const approvalTurn = crypto.randomUUID()
  const approvalChat = crypto.randomUUID()
  await pageEval(`window.__TAURI_INTERNALS__.invoke('begin_team', { id:${JSON.stringify(teamId)}, limit:3 })`)
  await pageEval(`(async () => {
    const { startTurn } = await import('/src/lib/ipc.ts'); window.__approvalEvents = [];
    await startTurn(${JSON.stringify({ turnId:approvalTurn, chatId:approvalChat, backend:'nova', prompt:'fixture-permission', trustWorkspace:true, teamId })}, (event) => window.__approvalEvents.push(event))
  })()`)
  await waitFor(`window.__approvalEvents.some((event) => event.type === 'approvalRequired')`, 'Backend permission did not arrive')
  assert.equal(await pageEval(`window.__approvalEvents.some((event) => event.type === 'turnFinished')`), false)
  const competing = { turnId:crypto.randomUUID(), chatId:crypto.randomUUID(), backend:'local', prompt:'competing writer', trustWorkspace:true, teamId }
  assert.match(await pageEval(`(async () => { const { startTurn } = await import('/src/lib/ipc.ts'); try { await startTurn(${JSON.stringify(competing)}, () => {}); return 'unexpected success' } catch (error) { return error.message } })()`), /single-writer/)
  await pageEval(`window.__TAURI_INTERNALS__.invoke('set_auto_approve', { enabled:true })`)
  await waitFor(`window.__approvalEvents.some((event) => event.type === 'turnFinished')`, 'Toggle did not release the pending permission')
  assert.ok(await pageEval(`window.__approvalEvents.some((event) => event.type === 'approvalResolved')`))
  const questionTurn = crypto.randomUUID()
  await pageEval(`(async () => { const { startTurn } = await import('/src/lib/ipc.ts'); window.__approvalEvents = [];
    await startTurn(${JSON.stringify({ turnId:questionTurn, chatId:approvalChat, backend:'nova', prompt:'fixture-question', trustWorkspace:true, teamId })}, (event) => window.__approvalEvents.push(event)) })()`)
  await waitFor(`window.__approvalEvents.some((event) => event.type === 'approvalRequired')`, 'Question did not arrive')
  assert.equal(await pageEval(`window.__approvalEvents.some((event) => event.type === 'approvalResolved')`), false, 'Auto-approve answered a question')
  await pageEval(`window.__TAURI_INTERNALS__.invoke('cancel_turn', { turnId:${JSON.stringify(questionTurn)} })`)
  await waitFor(`window.__approvalEvents.some((event) => event.type === 'turnFinished' && event.reason === 'cancelled')`, 'Cancellation while waiting did not finish')
  assert.ok(await pageEval(`window.__approvalEvents.some((event) => event.type === 'approvalResolved')`))
  await pageEval(`window.__TAURI_INTERNALS__.invoke('finish_team', { id:${JSON.stringify(teamId)} })`)
  await pageEval(`window.__TAURI_INTERNALS__.invoke('set_auto_approve', { enabled:false })`)
  const scrollTask = (await call('create_task', { title:'Scroll preservation fixture' })).taskId
  await call('open_task', { taskId:scrollTask })
  await call('start_task', { taskId:scrollTask, prompt:'scroll fixture\n'.repeat(120) })
  await eventually(async () => {
    const task = await call('read_task', { taskId:scrollTask })
    assert.ok(task.messages.some((message) => message.text.startsWith('fixture response scroll fixture')))
  }, 'Scroll fixture did not finish')
  assert.ok(await pageEval(`(() => { const log = document.querySelector('.chat-log'); return log.scrollHeight > log.clientHeight })()`))
  await pageEval(`(() => { const log = document.querySelector('.chat-log'); log.scrollTop = 0; log.dispatchEvent(new Event('scroll', { bubbles:true })) })()`)
  await call('send_message', { taskId:scrollTask, text:'scroll follow-up' })
  await eventually(async () => {
    const task = await call('read_task', { taskId:scrollTask })
    assert.ok(task.messages.some((message) => message.text === 'fixture response scroll follow-up'))
  }, 'Scroll follow-up did not finish')
  assert.equal(await pageEval(`document.querySelector('.chat-log').scrollTop`), 0, 'Incoming output moved a reader away from older messages')
  await call('archive_task', { taskId:scrollTask })
  await pageEval(`document.querySelector('.rail-settings').click()` )
  await waitFor(`Boolean(document.querySelector('.settings-nav'))`, 'Settings did not open')
  await pageEval(`Array.from(document.querySelectorAll('.settings-nav button')).find((button) => button.textContent.includes('Tools'))?.click()` )
  await waitFor(`Boolean(document.querySelector('.settings-stage[aria-label=Tools]'))`, 'Tools settings did not open')
  assert.equal(await pageEval(`Boolean(document.querySelector('#nova-integrations[open]'))`), false, 'Tools automatically opened the integrations sheet')
  await pageEval(`(() => { const button = Array.from(document.querySelectorAll('.settings-stage button')).find((button) => button.textContent === 'Manage'); button?.focus(); button?.click() })()` )
  await waitFor(`Boolean(document.querySelector('#nova-integrations:modal'))`, 'Manage did not open a modal sheet')
  await pageEval(`document.querySelector('#nova-integrations button').click()` )
  await waitFor(`!document.querySelector('#nova-integrations')`, 'Sheet did not dismiss')
  await waitFor(`document.activeElement?.textContent === 'Manage'`, 'Dismissal did not return focus')
  await pageEval(`Array.from(document.querySelectorAll('.settings-stage button')).find((button) => button.textContent === 'Manage')?.click()`)
  await waitFor(`Boolean(document.querySelector('#nova-integrations:modal'))`, 'Sheet did not reopen')
  await evaluate(webview, null, 'Input.dispatchKeyEvent', { type:'keyDown', key:'Escape', code:'Escape', windowsVirtualKeyCode:27 })
  await evaluate(webview, null, 'Input.dispatchKeyEvent', { type:'keyUp', key:'Escape', code:'Escape', windowsVirtualKeyCode:27 })
  await waitFor(`!document.querySelector('#nova-integrations')`, 'Escape did not dismiss the sheet')
  await waitFor(`document.activeElement?.textContent === 'Manage'`, 'Escape did not return focus')
  // Emulation overrides belong to the debugger connection that sets them.
  motionSession = new WebSocket(webview)
  await once(motionSession, 'open', { signal:AbortSignal.timeout(5000) })
  const motionReply = once(motionSession, 'message', { signal:AbortSignal.timeout(5000) })
  motionSession.send(JSON.stringify({ id:1, method:'Emulation.setEmulatedMedia', params:{ features:[{ name:'prefers-reduced-motion', value:'reduce' }] } }))
  assert.ok(!JSON.parse((await motionReply)[0].data).error)
  await waitFor(`matchMedia('(prefers-reduced-motion: reduce)').matches`, 'Reduced-motion emulation did not take effect')
  await pageEval(`Array.from(document.querySelectorAll('.sidebar button, .new-task')).find((button) => button.textContent.includes('Teams'))?.click()`)
  await waitFor(`Boolean(document.querySelector('[aria-label="Teams workspace"]'))`, 'Teams workspace did not open')
  await pageEval(`document.querySelector('.teams-workspace summary').click()`)
  await waitFor(`Boolean(document.querySelector('.teams-workspace .crew-profile'))`, 'Team configuration did not expand')
  assert.ok(await pageEval(`Boolean(document.querySelector('textarea'))`), 'Teams composer is missing')
  console.log(`Called and checked all ${tested.size} advertised tools through the Windows app and MCP shim.`)
} finally {
  motionSession?.close()
  shim?.stdin.end()
  if (app && app.exitCode === null) {
    app.kill()
    await once(app, 'exit')
  }
  await rm(join(tmpdir(), `nova-desktop-tools-${app?.pid}.json`), { force: true })
  await vite.close()
  await new Promise((resolve) => site.close(resolve))
  await rm(root, { recursive: true, force: true, maxRetries: 6, retryDelay: 250 }).catch(() => {})
}
