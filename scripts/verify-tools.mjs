import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { createServer as httpServer } from 'node:http'
import { copyFile, mkdtemp, mkdir, readFile, readdir, writeFile, rm } from 'node:fs/promises'
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
    const response = await new Promise((resolve, reject) => {
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
    if (method !== 'Runtime.evaluate') return response
    const { result, exceptionDetails } = response
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text)
    return result?.value
  } finally {
    socket.close()
  }
}

const packagedFiles = process.argv.includes('--packaged-files')
const root = await mkdtemp(join(tmpdir(), 'nova-tools-smoke-'))
// A concurrent development build must not replace or lock the running fixture.
const debug = resolve('src-tauri/target/debug')
for (const name of ['novacode-desktop.exe', 'nova-tools.exe', ...(await readdir(debug)).filter((name) => name.endsWith('.dll'))]) {
  await copyFile(join(debug, name), join(root, name === 'novacode-desktop.exe' ? 'desktop-probe.exe' : name))
}

const workspace = join(root, 'workspace')
const board = join(workspace, '.nova', 'moodboards', 'fixture')
const skill = join(workspace, '.nova', 'skills', 'fixture')
await mkdir(board, { recursive: true })
await mkdir(skill, { recursive: true })
await mkdir(join(root, 'profile'))
await mkdir(join(root, 'appdata'))
await mkdir(join(root, 'localappdata'))
await writeFile(join(workspace, 'fixture.txt'), 'desktop fixture needle\n')
await writeFile(join(workspace, 'editor.ts'), 'const greeting: string = "hello";\r\nconsole.log(greeting);\r\n')
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
  if (frame.content === 'fixture-permission' || frame.content === 'fixture-question' || frame.content === 'fixture-edit') {
    pending = frame
    emit({ type:'approval_required', request_id:frame.id, interrupt_id:'same-id',
      kind:frame.content === 'fixture-question' ? 'question' : 'tool', payload:frame.content === 'fixture-edit' ? {action_requests:[{name:'write_file'}]} : { question:'Fixture question?', tool:'fixture' } }); return
  }
  if (frame.content === 'fixture-activity') {
    emit({ type:'reasoning_delta', request_id:frame.id, content:'Checking the local server, then opening the page.' });
    setTimeout(() => {
      for (let i = 0; i < 16; i++) {
        emit({ type:'tool_call', request_id:frame.id, tool_call_id:'activity-' + i, tool:i % 3 ? 'eval' : 'Desktop · open_browser', args:{ command:'fixture ' + i } });
        emit({ type:'tool_result', request_id:frame.id, tool_call_id:'activity-' + i, is_error:i < 3, output:i < 3 ? 'Fixture failure ' + i : 'Fixture success ' + i });
      }
      emit({ type:'text', request_id:frame.id, content:'Activity fixture finished.' });
      emit({ type:'done', request_id:frame.id, status:'completed', ok:true });
    }, 1500);
    return;
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
const vite = await createServer({ cacheDir:join(root, 'vite-cache'), server: { host: 'localhost', port: 1420, strictPort: true, hmr: false, watch:null } })
let app, shim, motionSession
try {
  await vite.listen()
  app = spawn(join(root, 'desktop-probe.exe'), [], {
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
    const target = targets.find(({ url }) => packagedFiles
      ? url.startsWith('http://tauri.localhost/') || url.startsWith('tauri://localhost/')
      : url.startsWith('http://localhost:1420/'))
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

  // Exercise actual WebView input so native popover focus and dismissal are tested.
  async function clickControl(selector) {
    await waitFor(`(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el || el.disabled) return false; const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x+r.width/2, r.y+r.height/2)) })()`, `Control is covered: ${selector}`)
    await pageEval(`document.querySelector(${JSON.stringify(selector)}).focus()`)
    const point = await pageEval(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x:r.x+r.width/2, y:r.y+r.height/2 } })()`)
    await evaluate(webview, null, 'Input.dispatchMouseEvent', { type:'mousePressed', button:'left', clickCount:1, ...point })
    await evaluate(webview, null, 'Input.dispatchMouseEvent', { type:'mouseReleased', button:'left', clickCount:1, ...point })
  }
  async function pressControl(key, code) {
    await evaluate(webview, null, 'Input.dispatchKeyEvent', { type:'keyDown', key, code:key, windowsVirtualKeyCode:code, ...(key === 'Enter' ? {text:'\r',unmodifiedText:'\r'} : {}) })
    await evaluate(webview, null, 'Input.dispatchKeyEvent', { type:'keyUp', key, code:key, windowsVirtualKeyCode:code })
  }
  async function selectApproval(mode) {
    await clickControl('.approval-trigger')
    await waitFor(`document.querySelector('.approval-menu').matches(':popover-open')`, 'Approval menu did not open')
    await clickControl(`.approval-menu [data-mode="${mode}"]`)
    await waitFor(`document.querySelector('.approval-trigger').dataset.mode === ${JSON.stringify(mode)} && !document.querySelector('.approval-menu').matches(':popover-open')`, 'Approval mode was not selected')
    await waitFor(`JSON.parse(localStorage.getItem('nova.desktop.settings.v1')).approvalMode === ${JSON.stringify(mode)}`, 'Approval mode was not persisted')
  }
  await evaluate(webview, null, 'Page.bringToFront', {})
  await waitFor(`!document.querySelector('#boot')`, 'Startup overlay did not close')
  await clickControl('.agents-trigger')
  await waitFor(`document.querySelector('.agents-card').matches(':popover-open')`, 'Agents popover did not open')
  assert.ok(await pageEval(`document.activeElement.classList.contains('agents-close')`), 'Popover did not focus its close action')
  assert.ok(await pageEval(`document.querySelector('.agents-empty').textContent.includes('Start a task')`), 'Missing empty-state guidance')
  await pressControl('Escape', 27)
  await waitFor(`!document.querySelector('.agents-card').matches(':popover-open')`, 'Escape did not dismiss agents')
  assert.ok(await pageEval(`document.activeElement.classList.contains('agents-trigger')`), 'Popover did not return focus')
  await clickControl('.agents-trigger')
  await clickControl('.composer textarea')
  await waitFor(`!document.querySelector('.agents-card').matches(':popover-open')`, 'Outside click did not dismiss agents')
  assert.equal(await pageEval(`document.activeElement.getAttribute('aria-label')`), 'Prompt', 'Popover swallowed the outside click')
  await pageEval(`document.querySelector('.view-tabs [aria-selected="true"]').focus()`)
  await pressControl('ArrowDown', 40)
  await waitFor(`document.querySelector('.view-tabs [aria-selected="true"]').getAttribute('aria-label') === 'Files'`, 'Tab arrow navigation failed')
  await pressControl('Home', 36)
  await waitFor(`document.querySelector('.view-tabs [aria-selected="true"]').getAttribute('aria-label') === 'Chat'`, 'Tab Home navigation failed')
  await clickControl('.approval-trigger')
  await waitFor(`document.querySelector('.approval-menu').matches(':popover-open')`, 'Approval menu did not open')
  assert.equal(await pageEval(`document.querySelectorAll('.approval-menu [role="menuitemradio"]').length`), 4)
  assert.equal(await pageEval(`document.querySelector('.approval-menu [data-mode="plan"]').getAttribute('aria-disabled')`), 'true', 'Plan must explain its Codex requirement')
  await pressControl('ArrowDown', 40)
  assert.equal(await pageEval(`document.activeElement.dataset.mode`), 'acceptEdits', 'Approval arrow navigation failed')
  await pressControl('Enter', 13)
  await waitFor(`document.querySelector('.approval-trigger').dataset.mode === 'acceptEdits'`, 'Keyboard approval selection failed')
  await selectApproval('fullAccess')
  await selectApproval('standard')
  for (const view of ['Files', 'GitHub', 'Board', 'Preview']) {
    await clickControl(`.view-tabs [aria-label="${view}"]`)
    await waitFor(`document.querySelector('.view-tabs [aria-label="${view}"]').getAttribute('aria-selected') === 'true'`, 'Rail view did not open')
    await clickControl(`.view-tabs [aria-label="${view}"]`)
    await waitFor(`document.querySelector('.view-tabs [aria-label="Chat"]').getAttribute('aria-selected') === 'true' && document.querySelector('.right').classList.contains('is-closed')`, 'Second rail click did not collapse the pane')
  }
  await clickControl('.personality-trigger')
  await waitFor(`document.querySelector('.personality-menu').matches(':popover-open')`, 'Personality menu did not open')
  assert.equal(await pageEval(`document.querySelectorAll('.personality-menu button > .crew-portrait').length`), 4, 'Individual personality avatars missing')
  assert.ok(await pageEval(`(async () => { const avatar = document.querySelector('.personality-menu button > .crew-portrait'); const url = getComputedStyle(avatar).backgroundImage.slice(5,-2); const image = new Image(); image.src = url; await image.decode(); const r=avatar.getBoundingClientRect(); return image.naturalWidth > 0 && r.width >= 24 && r.height >= 24 })()`), 'Avatar artwork did not load')
  assert.equal(await pageEval(`document.querySelectorAll('.personality-menu .crew-team-avatar .crew-portrait').length`), 4, 'Team avatar missing')
  await clickControl('.personality-menu [data-mode="serena"]')
  await waitFor(`document.querySelector('.personality-trigger > [data-portrait="serena"]') && document.querySelector('.personality-trigger').dataset.mode === 'serena'`, 'Selected avatar did not update')
  await clickControl('.model-trigger')
  await waitFor(`Boolean(document.querySelector('.model-picker'))`, 'Personality model picker is missing')
  await pageEval(`Array.from(document.querySelectorAll('.model-picker-harnesses button')).find(b=>b.textContent.includes('Claude Code')).click()`)
  await pageEval(`Array.from(document.querySelectorAll('.model-picker-results button')).find(b=>b.querySelector('strong')?.textContent==='Claude Sonnet').click()`)
  await waitFor(`JSON.parse(localStorage.getItem('nova.desktop.settings.v1')).crew.serena?.agent==='claude' && JSON.parse(localStorage.getItem('nova.desktop.settings.v1')).crew.serena?.model==='sonnet'`, 'Personality model selection was not saved')
  await pageEval(`document.querySelector('[aria-label="Close model picker"]').click()`)
  await clickControl('.personality-trigger')
  await pressControl('Home', 36)
  await pressControl('Enter', 13)
  await waitFor(`document.querySelector('.personality-trigger').dataset.mode === '' && !document.querySelector('.personality-menu').matches(':popover-open')`, 'Personality keyboard selection did not clear the role')

  await clickControl('.composer-attach')
  await waitFor(`document.querySelector('.attachment-menu').matches(':popover-open')`, 'Attachment menu did not open')
  assert.equal(await pageEval(`document.querySelectorAll('.attachment-menu [role="menuitem"]').length`), 4)
  await clickControl('.attachment-menu [data-kind="project"]')
  await waitFor(`document.querySelector('.view-tabs [aria-label="Files"]').getAttribute('aria-selected')==='true'`, 'Project attachment did not open Files')
  await call('open_file', {path:'fixture.txt'})
  await waitFor(`Boolean(document.querySelector('.cm-content'))`, 'Attachment file preview missing')
  await clickControl('.composer textarea')
  await evaluate(webview, null, 'Input.insertText', {text:'Keep this prompt'})
  await clickControl('.files-attach-bar button')
  await waitFor(`document.querySelector('.composer textarea').value==='Keep this prompt fixture.txt'`, 'Project attachment overwrote the draft or used the wrong path')
  await clickControl('.composer-attach')
  await clickControl('.attachment-menu [data-kind="sketch"]')
  await waitFor(`Boolean(document.querySelector('.sketch-dialog:modal'))`, 'Drawing canvas did not open')
  const rectangle = await pageEval(`document.querySelector('.sketch-dialog canvas').getBoundingClientRect().toJSON()`)
  const from={x:rectangle.x+40,y:rectangle.y+40}, to={x:rectangle.x+160,y:rectangle.y+100}
  await evaluate(webview,null,'Input.dispatchMouseEvent',{type:'mousePressed',button:'left',buttons:1,clickCount:1,...from})
  await evaluate(webview,null,'Input.dispatchMouseEvent',{type:'mouseMoved',button:'left',buttons:1,...to})
  await evaluate(webview,null,'Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',clickCount:1,...to})
  await waitFor(`!document.querySelector('.sketch-dialog .send').disabled`, 'Drawing was not recorded')
  await pageEval(`Array.from(document.querySelectorAll('.sketch-tools button')).find(b=>b.textContent==='Undo').click()`)
  assert.ok(await pageEval(`document.querySelector('.sketch-dialog .send').disabled`), 'Undo did not remove the stroke')
  await pageEval(`document.querySelector('.sketch-dialog canvas').focus()`)
  await pressControl(' ',32)
  await pressControl('ArrowRight',39)
  await pressControl('ArrowDown',40)
  await pressControl(' ',32)
  await waitFor(`!document.querySelector('.sketch-dialog .send').disabled`, 'Keyboard drawing did not record a stroke')
  await pageEval(`Array.from(document.querySelectorAll('.sketch-tools button')).find(b=>b.textContent==='Clear').click()`)
  assert.ok(await pageEval(`document.querySelector('.sketch-dialog .send').disabled`), 'Clear did not clear the drawing')
  await pageEval(`Array.from(document.querySelectorAll('.sketch-tools button')).find(b=>b.textContent==='Undo').click()`)
  assert.equal(await pageEval(`document.querySelector('.sketch-dialog .send').disabled`), false, 'Clearing the canvas could not be undone')
  await pageEval(`Array.from(document.querySelectorAll('.sketch-dialog header button')).find(b=>b.textContent==='Cancel').click()`)
  await waitFor(`document.querySelector('.sketch-dialog footer').textContent.includes('Discard this drawing?')`, 'Cancel did not protect the drawing')
  await pageEval(`Array.from(document.querySelectorAll('.sketch-dialog footer button')).find(b=>b.textContent==='Keep drawing').click()`)
  await writeFile('.temp/sketch-canvas.png',Buffer.from((await evaluate(webview,null,'Page.captureScreenshot',{format:'png'})).data,'base64'))
  await clickControl('.sketch-dialog .send')
  await waitFor(`!document.querySelector('.sketch-dialog') && document.querySelector('.composer textarea').value.includes('.nova/attachments/sketch-')`, 'Sketch did not save and attach')
  const attached = await pageEval(`document.querySelector('.composer textarea').value`)
  assert.ok(attached.startsWith('Keep this prompt fixture.txt '), 'Sketch replaced existing attachments')
  const savedPath=attached.split(' ').at(-1)
  const savedPng=await readFile(join(workspace,savedPath))
  assert.equal(savedPng.readUInt32BE(16),1000)
  assert.equal(savedPng.readUInt32BE(20),600)
  assert.match(savedPath,/^\.nova\/attachments\/sketch-[a-f0-9-]+\.png$/)
  const originalTask=(await call('list_tasks')).activeId
  const otherDraftTask=(await call('create_task',{title:'Other attachment draft'})).taskId
  await call('open_task',{taskId:otherDraftTask})
  await waitFor(`document.querySelector('.composer textarea').value===''`, 'Attachments leaked into a different task')
  await evaluate(webview,null,'Input.insertText',{text:'Other draft'})
  await call('open_task',{taskId:originalTask})
  await waitFor(`document.querySelector('.composer textarea').value===${JSON.stringify(attached)}`, 'Switching tasks lost the attached draft')
  const rootMismatch = await pageEval(`window.__TAURI_INTERNALS__.invoke('save_sketch',{root:'wrong-root',dataUrl:'data:image/png;base64,abc'}).then(()=> 'unexpected success',error=>error.message)`)
  assert.match(rootMismatch,/Workspace changed/)
  const badPng = await pageEval(`window.__TAURI_INTERNALS__.invoke('save_sketch',{root:${JSON.stringify(info.root)},dataUrl:'data:text/html;base64,PHNjcmlwdD4='}).then(()=> 'unexpected success',error=>error.message)`)
  assert.match(badPng,/PNG/)
  // Clear the fixture draft through the same input path a user edits.
  await clickControl('.composer textarea')
  await pageEval(`document.querySelector('.composer textarea').select()`)
  await evaluate(webview,null,'Input.insertText',{text:''})
  await pageEval(`document.querySelector('.view-tabs [aria-label="Chat"]').click()`)
  console.log('Avatar/model selection, rail toggles, attachment menu, project files, keyboard canvas, Undo, PNG persistence and write boundaries passed.')
  await pageEval(`window.__TAURI_INTERNALS__.invoke('set_approval_mode', {mode:'plan'})`)
  const planError = await pageEval(`window.__TAURI_INTERNALS__.invoke('start_turn', {...${JSON.stringify({turnId:crypto.randomUUID(),chatId:crypto.randomUUID(),backend:'nova',prompt:'Plan fixture',trustWorkspace:true})}, onEvent:'__CHANNEL__:' + window.__TAURI_INTERNALS__.transformCallback(() => {})}).then(() => 'unexpected success', error => error.message)`)
  assert.match(planError, /does not enforce read-only/)
  await pageEval(`window.__TAURI_INTERNALS__.invoke('set_approval_mode', {mode:'standard'})`)
  console.log('Four approval modes, Plan backend boundary, view-rail keyboard navigation, agents focus, Escape and click-through passed.')

  // Chromium may attempt style attributes while replacing editable text. Those
  // remain blocked; this regression checks admission of generated stylesheets.
  const observeStyles = `window.__styleBlocks = []; document.addEventListener('securitypolicyviolation', (event) => window.__styleBlocks.push({ directive:event.effectiveDirective, uri:event.blockedURI, source:event.sourceFile, line:event.lineNumber }))`

  async function editSource(text) {
    await pageEval(`Array.from(document.querySelectorAll('.source-toolbar button')).find((button) => button.textContent.trim() === 'Edit')?.click()`)
    await pageEval(`(() => { const content = document.querySelector('.cm-content'); content.focus(); document.execCommand('selectAll') })()`)
    await evaluate(webview, null, 'Input.insertText', { text })
    await waitFor(`document.querySelector('.source-status')?.textContent.includes('Unsaved')`, 'Edits were not tracked')
  }

  async function verifyFileEditor() {
    await call('open_file', { path: 'fixture.txt' })
    await waitFor('document.body.innerText.includes("desktop fixture needle")', 'File preview did not open')
    await call('open_file', { path:'editor.ts' })
    await waitFor(`document.querySelector('.cm-content')?.textContent.includes('const greeting')`, 'Editable source did not open')
    await waitFor(`document.querySelector('.source-status')?.textContent.includes('TypeScript') && Boolean(document.querySelector('.cm-line span'))`, 'Syntax highlighting did not load')
    assert.ok(await pageEval(`document.querySelector('.cm-lineNumbers').textContent.includes('1')`), 'Line numbers are missing')
    await editSource('const greeting: string = "edited";\nconsole.log(greeting);\n')
    await pageEval(`document.querySelector('.source-save').click()`)
    await waitFor(`document.querySelector('.source-status')?.textContent.startsWith('Saved')`, 'File did not save')
    assert.equal(await readFile(join(workspace, 'editor.ts'), 'utf8'), 'const greeting: string = "edited";\r\nconsole.log(greeting);\r\n', 'Save did not preserve CRLF')
    await editSource('const greeting = "local draft";\n')
    await writeFile(join(workspace, 'editor.ts'), 'const greeting = "external edit";\r\n')
    await waitFor(`document.querySelector('.source-message')?.textContent.includes('changed on disk')`, 'Disk edits were not detected')
    assert.ok(await pageEval(`document.querySelector('.cm-content').textContent.includes('local draft')`), 'Refresh destroyed unsaved edits')
    await pageEval(`document.querySelector('.source-save').click()`)
    await waitFor(`document.querySelector('.source-message.is-error')?.textContent.includes('changed on disk')`, 'Conflicting save was not refused')
    assert.equal(await readFile(join(workspace, 'editor.ts'), 'utf8'), 'const greeting = "external edit";\r\n')
    await call('open_file', { path:'fixture.txt' })
    await waitFor(`Boolean(document.querySelector('.unsaved-dialog:modal'))`, 'Navigation did not protect unsaved changes')
    await pageEval(`Array.from(document.querySelectorAll('.unsaved-dialog button')).find((button) => button.textContent === 'Keep editing').click()`)
    assert.ok(await pageEval(`document.querySelector('.cm-content').textContent.includes('local draft')`))
    // Repeating the same tool navigation after cancelling must still prompt.
    await call('open_file', { path:'fixture.txt' })
    await waitFor(`Boolean(document.querySelector('.unsaved-dialog:modal'))`, 'Repeated navigation bypassed the draft guard')
    await pageEval(`Array.from(document.querySelectorAll('.unsaved-dialog button')).find((button) => button.textContent === 'Discard changes').click()`)
    await waitFor(`document.querySelector('.cm-content')?.textContent.includes('desktop fixture needle')`, 'Discard did not navigate')
    await call('open_file', { path:'editor.ts' })
    await waitFor(`document.querySelector('.cm-content')?.textContent.includes('external edit')`, 'Discarded draft came back')
    await editSource('const recovered = true;\n')
    if (packagedFiles) assert.deepEqual(await pageEval(`window.__styleBlocks.filter((event) => event.directive !== 'style-src-attr')`), [], 'Editor actions blocked a packaged stylesheet')
    await pageEval(`location.reload()`)
    await waitFor(`Boolean(document.querySelector('.rail-settings'))`, 'App did not reload')
    await wait(500)
    if (packagedFiles) await pageEval(observeStyles)
    await call('open_file', { path:'editor.ts' })
    await waitFor(`Boolean(document.querySelector('.cm-content'))`, 'Reopened file did not render')
    await pageEval(`Array.from(document.querySelectorAll('.source-toolbar button')).find((button) => button.textContent.trim() === 'Edit')?.click()`)
    await waitFor(`document.querySelector('.cm-content')?.textContent.includes('const recovered')`, 'Unsaved draft did not survive reload')
    await pageEval(`document.querySelector('[aria-label="Close preview"]').click()`)
    await waitFor(`Boolean(document.querySelector('.unsaved-dialog:modal'))`, 'Close did not protect the recovered draft')
    await pageEval(`Array.from(document.querySelectorAll('.unsaved-dialog button')).find((button) => button.textContent === 'Save and continue').click()`)
    await waitFor(`!document.querySelector('.unsaved-dialog') && !document.querySelector('.preview-head')?.textContent.includes('editor.ts')`, 'Save and continue did not close the file')
    assert.equal(await readFile(join(workspace, 'editor.ts'), 'utf8'), 'const recovered = true;\r\n')
  }

  async function debugCommand(id, method, params) {
    const next = once(motionSession, 'message', { signal:AbortSignal.timeout(10000) })
    motionSession.send(JSON.stringify({ id, method, params }))
    const reply = JSON.parse((await next)[0].data)
    assert.equal(reply.id, id)
    assert.ok(!reply.error, JSON.stringify(reply.error))
    return reply.result
  }
  if (packagedFiles) {
    // This mode deliberately requires an embedded frontend; Vite has no production CSP.
    await writeFile(join(workspace, 'csp.ts'), Array.from({ length:30 }, (_, index) => `const value${index}: number = ${index};`).join('\n'))
    await mkdir(join(workspace, 'docs'))
    await writeFile(join(workspace, 'docs', 'start.md'), '# START_MARKER\n[Next file](next.md)\n')
    await writeFile(join(workspace, 'docs', 'next.md'), '# NEXT_MARKER\nSecond file.\n')
    await pageEval(observeStyles)
    await call('open_file', { path:'csp.ts' })
    await waitFor(`Boolean(document.querySelector('.cm-line'))`, 'Packaged editor did not render')
    const layout = await pageEval(`(() => { const scroller = document.querySelector('.cm-scroller'); const line = document.querySelector('.cm-line'); return {
      scroller:getComputedStyle(scroller).display, gutters:getComputedStyle(document.querySelector('.cm-gutters')).display,
      top:line.getBoundingClientRect().top, bottom:scroller.getBoundingClientRect().bottom, blocked:window.__styleBlocks
    } })()`)
    assert.equal(layout.scroller, 'flex', 'Packaged CSP blocked CodeMirror layout styles')
    assert.equal(layout.gutters, 'flex', 'Packaged editor gutters are stacked')
    assert.ok(layout.top < layout.bottom, 'Code is laid out below the editor viewport')
    assert.deepEqual(layout.blocked.filter((event) => event.directive !== 'style-src-attr'), [], 'Packaged CSP refused an editor stylesheet')
    await waitFor(`Boolean(document.querySelector('.cm-line span'))`, 'Packaged syntax highlighting did not load')
    await writeFile('.temp/files-packaged-fixed.png', Buffer.from((await evaluate(webview, null, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
    await verifyFileEditor()

    await call('open_file', { path:'docs/start.md' })
    await waitFor(`document.querySelector('.preview-body')?.textContent.includes('START_MARKER')`, 'Initial Markdown did not render')
    await pageEval(`document.querySelector('.preview-body a').click()`)
    await waitFor(`document.querySelector('.preview-body')?.textContent.includes('NEXT_MARKER')`, 'Relative link did not navigate')
    await pageEval(`document.querySelector('.preview-head button[aria-pressed]').click()`)
    await waitFor(`Boolean(document.querySelector('.source-editor:not([hidden])'))`, 'Markdown source did not open')
    await editSource('# LOCAL_DRAFT')
    await pageEval(`document.querySelector('.file-tab button[title="docs/next.md"]').click()`)
    assert.equal(await pageEval(`Boolean(document.querySelector('.unsaved-dialog'))`), false, 'Clicking the active tab prompted to leave its own draft')
    assert.ok(await pageEval(`document.querySelector('.cm-content').textContent.includes('LOCAL_DRAFT')`), 'Active-tab click discarded its draft')
    await pageEval(`document.querySelector('.preview-crumbs button').click()`)
    await waitFor(`Boolean(document.querySelector('.unsaved-dialog:modal'))`, 'Breadcrumb did not protect the Markdown draft')
    await pageEval(`Array.from(document.querySelectorAll('.unsaved-dialog button')).find((button) => button.textContent === 'Discard changes').click()`)

    await waitFor(`!document.querySelector('.preview')`, 'Breadcrumb did not return to the tree')
    await pageEval(`document.querySelector('.file-tab button[title="docs/next.md"]').click()`)
    await waitFor(`document.querySelector('.preview-crumb.is-file')?.textContent.trim() === 'next.md'`, 'Returning from the tree reopened the wrong file')
    await waitFor(`document.querySelector('.preview-body')?.textContent.includes('NEXT_MARKER')`, 'Selected tab shows the wrong content')
    assert.deepEqual(await pageEval(`window.__styleBlocks.filter((event) => event.directive !== 'style-src-attr')`), [], 'Editor remounts blocked a packaged stylesheet')
    console.log('Packaged Files: CSP, visible code, highlighting, CRLF saves, conflicts, recovery and linked-file/tab navigation passed.')
  } else {
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
  await pageEval(`document.querySelector('.view-tabs [aria-label="Board"]').click()`)
  await clickControl('.agents-trigger')
  await waitFor(`document.querySelector('.agents-card li strong')?.textContent === 'Renamed fixture'`, 'Running task is missing from agents')
  assert.ok(await pageEval(`document.querySelector('.personality-trigger').disabled`), 'Personality must stay locked during a run')
  await clickControl('.agents-card li button')
  await waitFor(`document.querySelector('.view-tabs [aria-label="Chat"]').getAttribute('aria-selected') === 'true' && !document.querySelector('.agents-card').matches(':popover-open')`, 'Selecting an agent did not open its chat and dismiss the card')
  const changingPlan = await pageEval(`window.__TAURI_INTERNALS__.invoke('set_approval_mode', {mode:'plan'}).then(() => 'unexpected success', error => error.message)`)
  assert.match(changingPlan, /Stop the running turn/)
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
  await verifyFileEditor()
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
  const editTask = (await call('create_task', {title:'Edit approval fixture'})).taskId
  await call('open_task', {taskId:editTask})
  await selectApproval('acceptEdits')
  await call('start_task', {taskId:editTask, prompt:'fixture-edit'})
  await eventually(async () => {
    const task = await call('read_task', {taskId:editTask})
    assert.ok(task.messages.some(message => message.text === 'permission delivered'), JSON.stringify(task))
    assert.equal(task.state, 'idle')
  }, 'Accept edits did not approve a recognized Nova file edit')
  await selectApproval('standard')
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
  assert.equal(await pageEval(`document.querySelector('.approval-trigger').dataset.mode`), 'standard')
  assert.equal(await pageEval(`document.querySelector('.approval-trigger').disabled`), false, 'Approval mode must remain usable during a pending decision')
  assert.ok(await pageEval(`(() => { const mode = document.querySelector('.approval-trigger'); const rect = mode.getBoundingClientRect(); return mode.contains(document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2)) })()`), 'Approval mode is clipped or covered in the composer')
  await selectApproval('fullAccess')
  await waitFor(`JSON.parse(localStorage.getItem('nova.desktop.settings.v1')).approvalMode === 'fullAccess'`, 'Composer approval preference was not saved')
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
  await selectApproval('standard')
  await waitFor(`JSON.parse(localStorage.getItem('nova.desktop.settings.v1')).approvalMode === 'standard'`, 'Manual approval was not saved')
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
  await pageEval(`Array.from(document.querySelectorAll('.settings-nav button')).find((button) => button.textContent.includes('Schedules'))?.click()`)
  await waitFor(`Boolean(document.querySelector('select[aria-label="Approval mode"]'))`, 'Schedules approval setting did not open')
  for (const mode of ['fullAccess', 'standard']) {
    await pageEval(`(() => { const select = document.querySelector('select[aria-label="Approval mode"]'); select.value = ${JSON.stringify(mode)}; select.dispatchEvent(new Event('change', {bubbles:true})) })()`)
    await waitFor(`document.querySelector('.approval-trigger').dataset.mode === ${JSON.stringify(mode)}`, 'Settings did not update composer approval mode')
  }
  await pageEval(`Array.from(document.querySelectorAll('.settings-nav button')).find((button) => button.textContent.includes('Tools'))?.click()` )
  await waitFor(`Boolean(document.querySelector('.settings-stage[aria-label=Tools]'))`, 'Tools settings did not open')
  assert.equal(await pageEval(`Boolean(document.querySelector('#nova-integrations[open]'))`), false, 'Tools automatically opened the integrations sheet')
  await pageEval(`(() => { const button = Array.from(document.querySelectorAll('.settings-stage button')).find((button) => button.textContent === 'Manage integrations'); button?.focus(); button?.click() })()` )
  await waitFor(`Boolean(document.querySelector('#nova-integrations:modal'))`, 'Manage did not open a modal sheet')
  await pageEval(`document.querySelector('#nova-integrations button').click()` )
  await waitFor(`!document.querySelector('#nova-integrations')`, 'Sheet did not dismiss')
  await waitFor(`document.activeElement?.textContent === 'Manage integrations'`, 'Dismissal did not return focus')
  await pageEval(`Array.from(document.querySelectorAll('.settings-stage button')).find((button) => button.textContent === 'Manage integrations')?.click()`)
  await waitFor(`Boolean(document.querySelector('#nova-integrations:modal'))`, 'Sheet did not reopen')
  await evaluate(webview, null, 'Input.dispatchKeyEvent', { type:'keyDown', key:'Escape', code:'Escape', windowsVirtualKeyCode:27 })
  await evaluate(webview, null, 'Input.dispatchKeyEvent', { type:'keyUp', key:'Escape', code:'Escape', windowsVirtualKeyCode:27 })
  await waitFor(`!document.querySelector('#nova-integrations')`, 'Escape did not dismiss the sheet')
  await waitFor(`document.activeElement?.textContent === 'Manage integrations'`, 'Escape did not return focus')
  // Emulation overrides belong to the debugger connection that sets them.
  motionSession = new WebSocket(webview)
  await once(motionSession, 'open', { signal:AbortSignal.timeout(5000) })
  const motionReply = once(motionSession, 'message', { signal:AbortSignal.timeout(5000) })
  motionSession.send(JSON.stringify({ id:1, method:'Emulation.setEmulatedMedia', params:{ features:[{ name:'prefers-reduced-motion', value:'reduce' }] } }))
  assert.ok(!JSON.parse((await motionReply)[0].data).error)
  await waitFor(`matchMedia('(prefers-reduced-motion: reduce)').matches`, 'Reduced-motion emulation did not take effect')
  for (const section of ['General', 'Appearance', 'Voice', 'Agents', 'Schedules', 'Tools', 'Connections', 'About']) {
    await pageEval(`Array.from(document.querySelectorAll('.settings-nav nav button')).find((button) => button.textContent.trim() === ${JSON.stringify(section)}).click()`)
    await waitFor(`document.querySelector('.settings-stage')?.getAttribute('aria-label') === ${JSON.stringify(section)}`, section + ' settings did not open')
    const controls = await pageEval(`Array.from(document.querySelectorAll('.settings-view button')).filter((button) => button.getClientRects().length).map((button) => ({ label:button.textContent.trim() || button.getAttribute('aria-label'), height:button.getBoundingClientRect().height, width:button.getBoundingClientRect().width, action:button.classList.contains('settings-button'), icon:!!button.querySelector('svg[aria-hidden="true"]') }))`)
    assert.ok(controls.length)
    for (const control of controls) {
      assert.ok(control.height >= 44 && control.width >= 44, section + ': undersized button ' + control.label)
      assert.ok(control.label, section + ': unnamed button')
      if (control.action) assert.ok(control.icon, section + ': missing action icon for ' + control.label)
    }
  }
  await pageEval(`Array.from(document.querySelectorAll('.settings-nav nav button')).find((button) => button.textContent.trim() === 'About').click()`)
  await pageEval(`Array.from(document.querySelectorAll('.settings-stage button')).find((button) => button.textContent === 'View notices').click()`)
  await waitFor(`Boolean(document.querySelector('#nova-notices:popover-open'))`, 'View notices did not open')
  await pageEval(`document.querySelector('#nova-notices button').click()`)
  await waitFor(`!document.querySelector('#nova-notices:popover-open')`, 'Notices close did not work')
  await pageEval(`Array.from(document.querySelectorAll('.settings-nav nav button')).find((button) => button.textContent.trim() === 'General').click()`)
  await waitFor(`document.querySelector('.settings-stage')?.getAttribute('aria-label') === 'General'`, 'General settings did not open')
  await writeFile('.temp/settings-buttons-desktop.png', Buffer.from((await debugCommand(2, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
  await debugCommand(3, 'Emulation.setDeviceMetricsOverride', { width:900, height:820, deviceScaleFactor:1, mobile:false })
  await waitFor(`innerWidth === 900`, 'Minimum window width did not apply')
  assert.ok(await pageEval(`(() => { const stage = document.querySelector('.settings-stage'); return stage.scrollWidth <= stage.clientWidth + 1 })()`), 'Settings overflow at the minimum window width')
  await writeFile('.temp/settings-buttons-compact.png', Buffer.from((await debugCommand(4, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
  await pageEval(`document.querySelector('.settings-back').click()`)
  await waitFor(`!document.querySelector('.settings-view')`, 'Back to workspace did not work')
  let resizeCommand = 20
  await debugCommand(resizeCommand++, 'Emulation.setDeviceMetricsOverride', { width:1600, height:1000, deviceScaleFactor:1, mobile:false })
  await pageEval(`Array.from(document.querySelectorAll('.view-tab[role="tab"]')).find((button) => button.textContent.trim() === 'Files' && button.getAttribute('aria-selected') !== 'true')?.click()`)
  await pageEval(`(() => {
    for (const [label, key, count] of [['Resize the task list', 'ArrowRight', 2], ['Resize the panel', 'ArrowLeft', 4], ['Resize the terminal', 'ArrowUp', 3]]) {
      for (let i = 0; i < count; i++) document.querySelector('[aria-label="' + label + '"]').dispatchEvent(new KeyboardEvent('keydown', { key, shiftKey:true, bubbles:true }));
    }
    const prompt = document.querySelector('textarea[aria-label="Prompt"]');
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(prompt, 'A long draft to test a short window.\\n'.repeat(14));
    prompt.dispatchEvent(new Event('input', { bubbles:true }));
  })()`)
  const preferredSizes = await pageEval(`['.rail', '.right', '.dock'].map((selector) => { const r = document.querySelector(selector).getBoundingClientRect(); return selector === '.dock' ? r.height : r.width })`)
  for (const [width, height] of [[900, 600], [1100, 700], [1600, 1000], [900, 820], [1280, 820]]) {
    await debugCommand(resizeCommand++, 'Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor:1, mobile:false })
    await waitFor(`innerWidth === ${width} && innerHeight === ${height}`, 'Window resize did not apply')
    for (const view of ['Chat', 'Files', 'Board', 'GitHub', 'Preview']) {
      await pageEval(`Array.from(document.querySelectorAll('.view-tab[role="tab"]')).find((button) => button.textContent.trim() === ${JSON.stringify(view)} && button.getAttribute('aria-selected') !== 'true')?.click()`)
      await waitFor(`document.querySelector('.view-tab.is-on')?.textContent.trim() === ${JSON.stringify(view)}`, 'View did not change')
      const bounds = await pageEval(`(() => {
        const main = document.querySelector('.main'); const composer = document.querySelector('.composer');
        return { main:main.clientWidth, verticalOverflow:main.scrollHeight > main.clientHeight + 1, overflow:main.scrollWidth > main.clientWidth + 1,
          composer:composer.getBoundingClientRect().toJSON(),
          clipped:Array.from(composer.querySelectorAll('button, select, textarea')).filter((el) => el.getClientRects().length).filter((el) => {
            const r = el.getBoundingClientRect(); return r.left < 0 || r.right > innerWidth || r.top < 0 || r.bottom > innerHeight || !el.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
          }).map((el) => el.getAttribute('aria-label') || el.textContent.trim()) }
      })()`)
      assert.equal(bounds.verticalOverflow, false, `${width}x${height} ${view}: workspace exceeds window height`)
      assert.equal(bounds.overflow, false, `${width}x${height} ${view}: transcript overflows ${JSON.stringify(bounds)}`)
      assert.deepEqual(bounds.clipped, [], `${width}x${height} ${view}: composer controls clipped`)
    }
    await pageEval(`document.querySelector('[aria-label="Maximize the terminal"]').click()`)
    await waitFor(`Boolean(document.querySelector('.dock.is-max'))`, 'Terminal did not maximize')
    assert.ok(await pageEval(`(() => { const main = document.querySelector('.main'); return main.scrollHeight <= main.clientHeight + 1 })()`), `${width}x${height}: maximized terminal pushes composer out of the window`)
    await pageEval(`document.querySelector('[aria-label="Restore terminal size"]').click()`)
    await waitFor(`!document.querySelector('.dock.is-max')`, 'Terminal did not restore')
    if (width === 1600) {
      const restored = await pageEval(`['.rail', '.right', '.dock'].map((selector) => { const r = document.querySelector(selector).getBoundingClientRect(); return selector === '.dock' ? r.height : r.width })`)
      assert.deepEqual(restored, preferredSizes, 'Panel sizes were lost after shrinking and growing the window')
    }
    await pageEval(`Array.from(document.querySelectorAll('.view-tab[role="tab"]')).find((button) => button.textContent.trim() === 'Board' && button.getAttribute('aria-selected') !== 'true')?.click()`)
    await writeFile(`.temp/responsive-${width}x${height}.png`, Buffer.from((await debugCommand(resizeCommand++, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
  }
  await waitFor(`Array.from(document.querySelectorAll('.splitter')).every((el) => { const rect = el.parentElement.getBoundingClientRect(); const size = el.classList.contains('splitter-x') ? rect.width : rect.height; return Math.abs(Number(el.getAttribute('aria-valuenow')) - size) <= 1 })`, 'Resize handles do not report the visible pane sizes')
  const fittedDockHeight = await pageEval(`document.querySelector('.dock').getBoundingClientRect().height`)
  await pageEval(`document.querySelector('[aria-label="Resize the terminal"]').dispatchEvent(new KeyboardEvent('keydown', { key:'ArrowDown', bubbles:true }))`)
  await waitFor(`document.querySelector('.dock').getBoundingClientRect().height < ${fittedDockHeight - 10}`, 'Keyboard resize did not start from the visible terminal height')
  await pageEval(`(() => { const prompt = document.querySelector('textarea[aria-label="Prompt"]'); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(prompt, ''); prompt.dispatchEvent(new Event('input', { bubbles:true })); document.querySelector('[aria-label="Settings"]').click() })()`)
  await waitFor(`Boolean(document.querySelector('.settings-view'))`, 'Settings did not open for resize checks')
  for (const [width, height] of [[900, 600], [1280, 820]]) {
    await debugCommand(resizeCommand++, 'Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor:1, mobile:false })
    for (const section of ['Recommended', 'General', 'Appearance', 'Voice', 'Agents', 'Schedules', 'Tools', 'Connections', 'About']) {
      await pageEval(`Array.from(document.querySelectorAll('.settings-nav nav button')).find((button) => button.textContent.trim() === ${JSON.stringify(section)}).click()`)
      await waitFor(`document.querySelector('.settings-stage')?.getAttribute('aria-label') === ${JSON.stringify(section)}`, 'Settings did not change')
      const overflow = await pageEval(`Array.from(document.querySelectorAll('.settings-stage, .settings-group, .settings-row')).filter((el) => el.scrollWidth > el.clientWidth + 1).map((el) => el.className + ': ' + el.textContent.trim().slice(0, 90))`)
      assert.deepEqual(overflow, [], `${width}x${height} ${section}: settings content clipped`)
    }
    await writeFile(`.temp/responsive-settings-${width}x${height}.png`, Buffer.from((await debugCommand(resizeCommand++, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
  }
  await pageEval(`document.querySelector('.settings-back').click()`)
  const activityTask = (await call('create_task', { title:'Condensed activity fixture' })).taskId
  await call('open_task', { taskId:activityTask })
  await pageEval(`Array.from(document.querySelectorAll('.view-tab[role="tab"]')).find((button) => button.textContent.trim() === 'Chat').click()`)
  await call('start_task', { taskId:activityTask, prompt:'fixture-activity' })
  await waitFor(`document.querySelector('.thinking-panel summary')?.textContent.includes('Thinking')`, 'Thinking UI did not appear')
  await waitFor(`document.querySelector('.activity-summary')?.textContent.includes('16 tools')`, 'Tool burst was not grouped')
  assert.equal(await pageEval(`document.querySelectorAll('.activity-group').length`), 1)
  assert.equal(await pageEval(`document.querySelector('.activity-group').open`), false, 'Activity should start collapsed')
  assert.ok(await pageEval(`document.querySelector('.activity-summary').textContent.includes('3 failed')`), 'Collapsed activity hides failures')
  assert.ok(await pageEval(`document.querySelector('.activity-group').getBoundingClientRect().height < 65`), 'Collapsed activity is too tall')
  await writeFile('.temp/activity-collapsed.png', Buffer.from((await debugCommand(resizeCommand++, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
  await pageEval(`document.querySelector('.activity-summary').focus()`)
  await debugCommand(resizeCommand++, 'Input.dispatchKeyEvent', { type:'keyDown', key:'Enter', code:'Enter', text:'\r', unmodifiedText:'\r', windowsVirtualKeyCode:13 })
  await debugCommand(resizeCommand++, 'Input.dispatchKeyEvent', { type:'keyUp', key:'Enter', code:'Enter', windowsVirtualKeyCode:13 })
  await waitFor(`document.querySelector('.activity-group').open`, 'Keyboard could not expand activity')
  assert.equal(await pageEval(`document.querySelectorAll('.tool-row').length`), 16)
  assert.ok(await pageEval(`(() => { const log = document.querySelector('.chat-log').getBoundingClientRect(); const summary = document.querySelector('.activity-summary').getBoundingClientRect(); return summary.top >= log.top && summary.bottom <= log.bottom })()`), 'Expanding activity scrolled its header out of view')
  await pageEval(`document.querySelector('.thinking-panel summary').click()`)
  assert.ok(await pageEval(`document.querySelector('.thinking-text').textContent.includes('Checking the local server')`), 'Thinking text was not retained')
  await pageEval(`document.querySelector('.tool-row summary').click()`)
  assert.ok(await pageEval(`document.querySelector('.tool-row').open && document.querySelector('.tool-row-detail').textContent.includes('Fixture failure 0')`), 'Tool output was lost')
  await writeFile('.temp/activity-expanded.png', Buffer.from((await debugCommand(resizeCommand++, 'Page.captureScreenshot', { format:'png', captureBeyondViewport:false })).data, 'base64'))
  console.log('Condensed activity, failure counts, keyboard disclosure, thinking text and tool output passed.')
  await pageEval(`Array.from(document.querySelectorAll('.sidebar button, .new-task')).find((button) => button.textContent.includes('Teams'))?.click()`)
  await waitFor(`Boolean(document.querySelector('[aria-label="Teams workspace"]'))`, 'Teams workspace did not open')
  await pageEval(`document.querySelector('.teams-workspace summary').click()`)
  await waitFor(`Boolean(document.querySelector('.teams-workspace .crew-profile'))`, 'Team configuration did not expand')
  await clickControl('.model-trigger')
  await waitFor(`document.querySelectorAll('.model-picker-roles button').length===4`, 'Team member model controls missing')
  await pageEval(`Array.from(document.querySelectorAll('.model-picker-roles button')).find(b=>b.textContent==='Ralph').click()`)
  await waitFor(`Array.from(document.querySelectorAll('.model-picker-roles button')).some(b=>b.textContent==='Ralph' && b.getAttribute('aria-pressed')==='true')`, 'Switching team member closed the picker')
  await pageEval(`Array.from(document.querySelectorAll('.model-picker-harnesses button')).find(b=>b.textContent.includes('Claude Code')).click()`)
  await pageEval(`Array.from(document.querySelectorAll('.model-picker-results button')).find(b=>b.querySelector('strong')?.textContent==='Claude Opus').click()`)
  await waitFor(`JSON.parse(localStorage.getItem('nova.desktop.settings.v1')).crew.ralph?.model==='opus'`, 'Team member model was not configured')
  await pageEval(`document.querySelector('[aria-label="Close model picker"]').click()`)
  const teamForm = await pageEval(`(() => { const root = document.querySelector('.teams-workspace .team-members'); const row = root.querySelector('.settings-row'); const strong = row.querySelector('strong').getBoundingClientRect(); const small = row.querySelector('small').getBoundingClientRect(); const select = root.querySelector('select').getBoundingClientRect(); return {maxWidth:root.getBoundingClientRect().width, labelsSeparated:small.top >= strong.bottom, selectHeight:select.height, overflow:root.scrollWidth > root.clientWidth} })()`)
  assert.ok(teamForm.maxWidth <= 960 && teamForm.labelsSeparated && teamForm.selectHeight >= 44 && !teamForm.overflow, `Inline Teams form is malformed: ${JSON.stringify(teamForm)}`)
  assert.ok(await pageEval(`Boolean(document.querySelector('textarea'))`), 'Teams composer is missing')
  console.log(`Called and checked all ${tested.size} advertised tools through the Windows app and MCP shim; resize checks passed across five window sizes, five workspace views and nine Settings sections.`)
  }
} finally {
  motionSession?.close()
  shim?.stdin.end()
  if (app && app.exitCode === null) {
    app.kill()
    await Promise.race([once(app, 'exit').catch(() => {}), wait(5000)])
  }
  await rm(join(tmpdir(), `nova-desktop-tools-${app?.pid}.json`), { force: true })
  await vite.close()
  await new Promise((resolve) => site.close(resolve))
  await rm(root, { recursive: true, force: true, maxRetries: 6, retryDelay: 250 }).catch(() => {})
}
