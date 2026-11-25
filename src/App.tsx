import { useCallback, useEffect, useRef, useState } from 'react'

import { ChatLog } from './components/ChatLog'
import { Composer } from './components/Composer'
import { Interrupt } from './components/Interrupt'
import { PanelRail } from './components/PanelRail'
import { RightPanel } from './components/RightPanel'
import { Settings, type Section } from './components/Settings'
import { Sidebar } from './components/Sidebar'
import { Sky } from './components/Sky'
import { TerminalDock, type Dock } from './components/TerminalDock'
import { appendUser } from './lib/chat'
import { CREW, addressedCrew, crewPrompt, ralphRetry, systemPrompt, teamOrder, type CrewMode, type CrewRole, type Handoff } from './lib/crew'
import { clamp, defaultPanelWidth, defaultRailWidth, dockMax, dockMin, panelMax, panelMin, railCollapsed, railMax, railMin } from './lib/layout'
import { withViewTransition } from './lib/motion'
import type { AgentEvent, FinishReason, InterruptReply, WorkspaceInfo } from './lib/events'
import type { Pane } from './lib/panes'
import { cancelTurn, errorText, openNovaTerminal, pickWorkspace, respondApproval, setWorkspace, startTurn } from './lib/ipc'
import { MAX_CHATS, createChat, load, routeEvent, save, taskModel, titleFromPrompt, type Chat } from './lib/sessions'
import { AGENTS, applyAppearance, loadPreferences, recommendedAgent, savePreferences, type ModelSelection, type Preferences } from './lib/settings'
import { allNovaSlashCommands, desktopSlashCommands, novaSlashCommands } from './lib/slash'

const WORKSPACE_KEY = 'nova.desktop.workspace'
type Running = { turnId: string; chatId: string; role?: CrewRole; iteration?: number }
type InterruptRequest = Extract<AgentEvent, { type: 'approvalRequired' }>

export default function App() {
  const [workspace, setFolder] = useState<WorkspaceInfo | null>(null)
  const [restoredPath] = useState(() => {
    try { return loadPreferences().restoreFolder ? localStorage.getItem(WORKSPACE_KEY) : null }
    catch { return null }
  })
  const [folderBusy, setFolderBusy] = useState(Boolean(restoredPath))
  const [chats, setChats] = useState(() => {
    const restored = load()
    return restored.length ? restored : [createChat()]
  })
  const [activeId, setActiveId] = useState(() => chats[0].id)
  const [preferences, setPreferences] = useState(loadPreferences)
  const [settings, setSettings] = useState(false)
  const [settingsPage, setSettingsPage] = useState<{ section: Section; tab: string }>({ section: 'Appearance', tab: 'Harnesses' })
  const [terminalCommand, setTerminalCommand] = useState<{ chatId: string; text: string } | null>(null)
  const [running, setRunning] = useState<Running | null>(null)
  const live = useRef<Running | null>(null)
  const crewRun = useRef<{ cancelled: boolean } | null>(null)
  const replying = useRef(false)
  const [notice, setNotice] = useState<string | null>(null)
  const [offeredWorkspace, setOfferedWorkspace] = useState<string | null>(null)
  const [pane, setPane] = useState<Pane>('Changes')
  const [panelOpen, setPanelOpen] = useState(true)
  const [dock, setDock] = useState<Dock>('closed')
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [railWidth, setRailWidth] = useState(defaultRailWidth)
  const [panelWidth, setPanelWidth] = useState(defaultPanelWidth)
  const [dockHeight, setDockHeight] = useState<number | null>(null)
  const [interrupts, setInterrupts] = useState<InterruptRequest[]>([])
  const [replyPending, setReplyPending] = useState(false)
  const active = chats.find((chat) => chat.id === activeId) ?? chats[0]
  const agent = active.agent ?? recommendedAgent(preferences, workspace?.root)
  const backendName = AGENTS[agent]?.name ?? agent
  const commands = agent === 'nova' ? allNovaSlashCommands : desktopSlashCommands
  const wrongFolder = Boolean(active.workspace && workspace?.root !== active.workspace)
  const lastItem = active.items.at(-1)
  const lastPrompt = active.items.at(-2)
  const needsWorkspaceApproval = workspace && active.workspace === workspace.root && active.agent === 'nova'
    && lastPrompt?.kind === 'user' && lastItem?.kind === 'error'
    && (lastItem.errorKind === 'workspaceNotApproved'
      // Existing tasks may have saved the startup failure before it had a kind.
      || lastItem.text.startsWith('PermissionError: Workspace is not approved.'))
  const offerWorkspaceApproval = workspace && workspace.root === offeredWorkspace
    && !preferences.trustedWorkspaces.includes(workspace.root)

  useEffect(() => {
    const resize = () => {
      setRailWidth((width) => clamp(width, railMin, railMax()))
      setPanelWidth((width) => clamp(width, panelMin, panelMax()))
      setDockHeight((height) => height === null ? null : clamp(height, dockMin, dockMax()))
    }
    window.addEventListener('resize', resize)
    return () => window.removeEventListener('resize', resize)
  }, [])

  const update = useCallback((patch: Partial<Preferences>) => {
    setPreferences((current) => ({ ...current, ...patch }))
  }, [])

  useEffect(() => {
    const color = matchMedia('(prefers-color-scheme: light)')
    const paint = () => applyAppearance(preferences, color.matches)
    paint()
    color.addEventListener('change', paint)
    void Promise.resolve().then(() => savePreferences(preferences))
      .catch((err) => setNotice('Could not save settings: ' + errorText(err)))
    return () => color.removeEventListener('change', paint)
  }, [preferences])

  useEffect(() => {
    if (!restoredPath) return
    void setWorkspace(restoredPath).then(setFolder)
      .catch((err) => setNotice(errorText(err))).finally(() => setFolderBusy(false))
  }, [restoredPath])

  useEffect(() => {
    const timer = setTimeout(() => save(chats), 400)
    const flush = () => save(chats)
    window.addEventListener('pagehide', flush)
    return () => { clearTimeout(timer); window.removeEventListener('pagehide', flush) }
  }, [chats])

  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key === ',') {
        event.preventDefault()
        setSettings((open) => !open)
      } else if (event.ctrlKey && !event.altKey && event.code === 'Backquote') {
        event.preventDefault()
        setSettings(false)
        setDock((current) => current === 'closed' ? 'open' : 'closed')
      } else if (event.key === 'Escape') {
        setSettings(false)
      }
    }
    window.addEventListener('keydown', shortcut)
    return () => window.removeEventListener('keydown', shortcut)
  }, [])

  const openFolder = async (path?: string) => {
    if (live.current || folderBusy) return
    setFolderBusy(true)
    try {
      const picked = await (path ? setWorkspace(path) : pickWorkspace())
      if (!picked) return
      setFolder(picked)
      setOfferedWorkspace(picked.root)
      setNotice(null)
      localStorage.setItem(WORKSPACE_KEY, picked.root)
      if (active.workspace && active.workspace !== picked.root && !path) {
        const chat = createChat()
        setChats((previous) => [chat, ...previous].slice(0, MAX_CHATS))
        setActiveId(chat.id)
      }
    } catch (err) {
      setNotice(errorText(err))
    } finally { setFolderBusy(false) }
  }

  const addTasks = (drafts: Chat[]) => {
    setChats((previous) => {
      const protectedTask = previous.find(({ id }) => id === live.current?.chatId)
      const next = [...drafts, ...previous.filter(({ id }) => id !== protectedTask?.id)]
      return protectedTask ? [...next.slice(0, MAX_CHATS - 1), protectedTask] : next.slice(0, MAX_CHATS)
    })
    setActiveId(drafts[0].id)
  }

  const approveWorkspace = (root: string) => {
    setOfferedWorkspace(null)
    if (!preferences.trustedWorkspaces.includes(root)) {
      update({ trustedWorkspaces: [...preferences.trustedWorkspaces, root] })
    }
  }

  const send = useCallback((prompt: string, chat: Chat, mode?: 'scheduled' | 'approveWorkspace') => {
    if (live.current || folderBusy || !workspace || (chat.workspace && chat.workspace !== workspace.root)) return false
    const fresh = mode === 'scheduled'
    const approvingWorkspace = mode === 'approveWorkspace'
    const backend = chat.agent ?? recommendedAgent(preferences, workspace.root)
    const model = taskModel(chat, preferences.models[backend])
    const chatId = chat.id
    const turnId = crypto.randomUUID()
    live.current = { turnId, chatId }
    setRunning(live.current)
    setInterrupts([])
    setNotice(null)
    const started: Chat = {
      ...chat,
      title: chat.title === 'New task' ? titleFromPrompt(prompt) : chat.title,
      agent: backend, workspace: workspace.root, model: model ?? null,
      items: approvingWorkspace ? chat.items.slice(0, -1) : appendUser(chat.items, prompt),
      state: 'running',
    }
    setChats((previous) => fresh ? [started, ...previous].slice(0, MAX_CHATS)
      : previous.map((item) => item.id === chatId ? started : item))
    if (fresh) setActiveId(chatId)

    const finish = () => {
      if (live.current?.turnId !== turnId) return
      live.current = null
      setRunning(null)
      setInterrupts([])
      replying.current = false
      setReplyPending(false)
    }
    let spoken = ''
    let awaitingWorkspaceApproval = false
    void startTurn({ turnId, chatId: chat.connectionId ?? chatId, prompt, backend, model,
      effort: chat.effort, session: chat.session,
      check: preferences.checks[workspace.root],
      trustWorkspace: approvingWorkspace || preferences.trustedWorkspaces.includes(workspace.root),
    }, (event) => {
      if (event.type === 'turnFinished') {
        finish()
        if (event.reason === 'completed' && preferences.speak && spoken.trim() && 'speechSynthesis' in window) {
          window.speechSynthesis.cancel()
          window.speechSynthesis.speak(new SpeechSynthesisUtterance(spoken.slice(0, 6000)))
        }
        if (event.reason === 'failed' && !awaitingWorkspaceApproval) {
          setNotice((message) => message ?? 'The turn failed. Check the task for details.')
        }
      } else if (event.type === 'turnError') {
        awaitingWorkspaceApproval = event.kind === 'workspaceNotApproved'
      } else if (event.type === 'approvalRequired') {
        setInterrupts((queue) => [...queue, event])
      } else if (event.type === 'notice') {
        setNotice(event.message)
      } else if (event.type === 'messageDelta') {
        spoken += event.text
      } else if (event.type === 'messageFinal' && !event.plain) {
        spoken += `${spoken ? '\n' : ''}${event.text}`
      }
      if (event.type !== 'notice') setChats((previous) => routeEvent(previous, chatId, event))
    }).catch((err) => {
      finish()
      setChats((previous) => routeEvent(previous, chatId, { type: 'turnError', message: errorText(err) }))
    })
    return true
  }, [folderBusy, preferences, workspace])

  useEffect(() => {
    if (!workspace || running) return
    const tick = () => {
      if (live.current) return
      const schedule = preferences.schedules.find((item) => item.workspace === workspace.root && item.at <= Date.now())
      if (!schedule) return
      const chat = { ...createChat(), agent: schedule.agent, workspace: workspace.root }
      const next = new Date(schedule.at)
      const now = new Date()
      next.setFullYear(now.getFullYear(), now.getMonth(), now.getDate())
      if (next <= now) next.setDate(next.getDate() + 1)
      if (send(schedule.prompt, chat, 'scheduled')) {
        update({ schedules: schedule.daily
          ? preferences.schedules.map((item) => item.id === schedule.id ? { ...item, at: next.getTime() } : item)
          : preferences.schedules.filter((item) => item.id !== schedule.id) })
      }
    }
    const timer = setInterval(tick, 5000)
    return () => clearInterval(timer)
  }, [workspace, running, preferences.schedules, send, update])

  const answer = async (reply: InterruptReply) => {
    const request = interrupts[0]
    const turn = live.current
    if (!request || !turn || replying.current) return
    replying.current = true
    setReplyPending(true)
    try {
      await respondApproval(turn.turnId, request.interruptId, reply)
      setInterrupts((queue) => queue.filter(({ interruptId }) => interruptId !== request.interruptId))
    } catch (err) {
      setNotice(errorText(err))
    } finally {
      if (live.current?.turnId === turn.turnId) {
        replying.current = false
        setReplyPending(false)
      }
    }
  }

  const closeSettings = () => {
    setSettings(false)
    requestAnimationFrame(() => document.querySelector<HTMLButtonElement>('.rail-settings')?.focus())
  }

  const openSettings = (section: Section = 'Appearance', tab = 'Harnesses') => {
    setSettingsPage({ section, tab })
    setSettings(true)
  }
  const stopTurn = () => {
    if (crewRun.current) crewRun.current.cancelled = true
    if (live.current) void cancelTurn(live.current.turnId).catch((error) => setNotice(errorText(error)))
  }
  const sendCrew = (task: string, mode: CrewMode) => {
    if (live.current || !workspace || folderBusy || wrongFolder) return false
    if (!preferences.trustedWorkspaces.includes(workspace.root)) {
      setOfferedWorkspace(workspace.root)
      return false
    }
    const chatId = active.id
    const roles: readonly CrewRole[] = mode === 'team' ? teamOrder : [mode]
    const history = active.items.flatMap((item) => {
      if (item.kind === 'user' && item.recipient === mode) {
        return ['User: ' + item.text.slice(0, 2000)]
      }
      if (item.kind === 'assistant' && !item.provisional && item.speaker &&
        (item.speaker.scope ?? (item.speaker.to ? 'team' : item.speaker.role)) === mode) {
        return [CREW[item.speaker.role].name + ': ' + item.text.slice(0, 2000)]
      }
      return []
    }).slice(-8).join('\n\n')
    const notes: Handoff[] = []
    const run = { cancelled: false }
    crewRun.current = run
    setNotice(null)
    setChats((previous) => previous.map((chat) => chat.id === chatId ? {
      ...chat, workspace: workspace.root, state: 'running', items: appendUser(chat.items, task, mode),
      title: chat.title === 'New task' ? titleFromPrompt(task) : chat.title,
    } : chat))

    void (async () => {
      for (const [index, role] of roles.entries()) {
        const final = index === roles.length - 1
        const configuration = preferences.crew[role]
        const backend = configuration?.agent ?? 'nova'
        for (let iteration = 1; !run.cancelled; iteration += 1) {
          const turnId = crypto.randomUUID()
          const speaker = { role, to: roles[index + 1], stage: turnId, scope: mode }
          live.current = { turnId, chatId, role, iteration }
          setRunning(live.current)
          setInterrupts([])
          setReplyPending(false)
          replying.current = false
          let reply = ''
          const reason = await new Promise<FinishReason>((resolve) => {
            void startTurn({
              turnId, chatId: turnId, backend, model: configuration?.model?.trim() || undefined,
              personality: { role, systemPrompt: configuration?.systemPrompt?.trim() || systemPrompt(role) },
              prompt: crewPrompt(role, task, history, notes, mode === 'team', final),
              trustWorkspace: true, check: final && role !== 'ralph' && role !== 'serena' && role !== 'nora'
                ? preferences.checks[workspace.root] : undefined,
            }, (event) => {
              if (event.type === 'turnStarted' && run.cancelled) void cancelTurn(turnId).catch(() => {})
              if (event.type === 'approvalRequired') setInterrupts((queue) => [...queue, event])
              if (event.type === 'messageDelta' || (event.type === 'messageFinal' && !event.plain)) {
                const separator = reply && event.type === 'messageFinal' ? '\n\n' : ''
                reply = (reply + separator + event.text).slice(-8000)
              }
              if (event.type !== 'notice' || event.level !== 'info') {
                setChats((previous) => routeEvent(previous, chatId, event, speaker))
              }
              if (event.type === 'turnFinished') resolve(event.reason)
            }).catch((error) => {
              setChats((previous) => routeEvent(previous, chatId, { type: 'turnError', message: errorText(error) }, speaker))
              resolve('failed')
            })
          })
          if (run.cancelled || reason === 'cancelled') {
            run.cancelled = true
            break
          }
          const retry = role === 'ralph' && reason === 'completed' && reply.trimEnd().endsWith(ralphRetry)
          notes.push({ from: role, text: reason === 'completed'
            ? reply.replaceAll(ralphRetry, '').trim() || 'No written handoff was returned.'
            : 'This stage failed. See the transcript for details and do not treat this work as verified.' })
          if (!retry || iteration >= 3) {
            if (retry) setNotice('Ralph reached the three-iteration limit. A check still needs attention.')
            break
          }
        }
        if (run.cancelled) break
      }
    })().finally(() => {
      crewRun.current = null
      live.current = null
      setRunning(null)
      setInterrupts([])
      setReplyPending(false)
      replying.current = false
      setChats((previous) => previous.map((chat) => chat.id === chatId ? { ...chat, state: 'idle' } : chat))
    }).catch((error) => setNotice(errorText(error)))
    return true
  }
  const launchTerminal = () => void openNovaTerminal().catch((error) => setNotice(errorText(error)))
  const submit = (text: string) => {
    const address = addressedCrew(text)
    if (address) return Boolean(address.task) && sendCrew(address.task, address.mode)
    const match = /^\/([\w?:-]+)(?:\s+([\s\S]+))?$/.exec(text)
    if (!match) return active.crew ? sendCrew(text, active.crew) : send(text, active)
    const name = match[1].toLowerCase()
    const args = match[2]
    const desktop = desktopSlashCommands.find((command) => command.name === name)
    if (desktop && !args) {
      switch (desktop.canonical) {
        case 'new': addTasks([createChat()]); return true
        case 'stop':
          if (running) stopTurn()
          else setNotice('No turn is running.')
          return true
        case 'settings':
        case 'theme': openSettings(); return true
        case 'auth': openSettings('Agents', 'Providers'); return true
        case 'mcp': openSettings('Connections'); return true
        case 'terminal':
          if (!workspace || folderBusy || wrongFolder) {
            setNotice('Open the task folder to use its terminal.')
            return false
          }
          setDock('open')
          return true
      }
    }
    if (agent !== 'nova') return send(text, active)
    const command = novaSlashCommands.find((entry) => entry.name === name || entry.name === name.replaceAll('_', '-'))
    if (command?.surface === 'terminal' || (!args && name === 'restore') || desktop?.canonical === 'mcp'
        || desktop?.canonical === 'model' || desktop?.canonical === 'auth') {
      if (!workspace || folderBusy || wrongFolder) {
        setNotice('Open the task folder to continue in Nova’s terminal.')
        return false
      }
      setNotice(null)
      setTerminalCommand({ chatId: active.id, text })
      return true
    }
    setTerminalCommand(null)
    return send(command ? `/${command.canonical}${args ? ` ${args}` : ''}` : text, active)
  }

  const busy = running?.chatId === active.id
  const interrupt = busy ? interrupts[0] : undefined
  const work = interrupt ? 'Waiting for approval'
    : lastItem?.kind === 'tool' && lastItem.ok === undefined ? `Running ${lastItem.label}`
      : lastItem?.kind === 'assistant' ? 'Writing response' : 'Working'
  const selectModel = ({ agent: nextAgent, model, effort }: ModelSelection) => {
    if (live.current?.chatId === active.id) return
    const switched = nextAgent !== agent
    const connectionId = switched ? crypto.randomUUID() : active.connectionId
    setChats((previous) => previous.map((chat) => chat.id === active.id ? {
      ...chat, agent: nextAgent, model, effort, connectionId,
      session: switched ? undefined : chat.session,
      items: switched ? chat.items.map((item) => item.kind === 'assistant' && !item.author
        ? { ...item, author: backendName } : item) : chat.items,
    } : chat))
    setPreferences((previous) => ({
      ...previous,
      recentModels: [{ agent: nextAgent, model, effort }, ...previous.recentModels.filter(
        (entry) => entry.agent !== nextAgent || entry.model !== model,
      )].slice(0, 12),
    }))
  }
  return (
    <>
      <div className="shell" hidden={settings}>
        <Sidebar chats={chats} activeId={active.id} workspace={workspace}
          open={sidebarOpen} onToggle={() => setSidebarOpen((open) => !open)}
          onSelect={(id) => withViewTransition(() => setActiveId(id))}
          width={railWidth} onWidth={setRailWidth}
          onRename={(id, title) => {
            if (title.trim()) setChats((previous) => previous.map((chat) =>
              chat.id === id ? { ...chat, title: title.trim().slice(0, 120) } : chat))
          }}
          onPin={(id) => setChats((previous) => previous.map((chat) =>
            chat.id === id ? { ...chat, pinned: !chat.pinned } : chat))}
          onDelete={(id) => {
            // Deleting under a live turn would leave the stream with nowhere to
            // land, so say why instead of closing the menu on a no-op.
            if (live.current?.chatId === id) {
              setNotice('Stop the running turn before deleting this task.')
              return
            }
            const index = chats.findIndex((chat) => chat.id === id)
            const remaining = chats.filter((chat) => chat.id !== id)
            if (!remaining.length) remaining.push(createChat())
            setChats(remaining)
            // The neighbour, not the top: deleting from the middle of the list
            // should not cost the reader their place.
            if (activeId === id) setActiveId(remaining[Math.min(index, remaining.length - 1)].id)
          }}
          onNew={() => addTasks([createChat()])} onOpenFolder={() => void openFolder()}
          onSettings={() => openSettings()} folderBusy={folderBusy || Boolean(running)} />

        <main className="main">
          {preferences.wallpaper && <Sky still={preferences.still} />}
          <ChatLog key={active.id} items={active.items} busy={busy}
            markdown={preferences.markdown} backend={backendName} workspaceReady={!!workspace} />
          {interrupt && <Interrupt key={interrupt.interruptId} request={interrupt} pending={replyPending}
            onReply={(reply) => void answer(reply)} />}
          {notice && <div className="notice" role="status">
            <span>{notice}</span><button type="button" className="ghost" onClick={() => setNotice(null)} aria-label="Dismiss notice">×</button>
          </div>}
          {wrongFolder && <div className="notice">
            <span>This task belongs to a different folder.</span>
            <button type="button" className="ghost" disabled={Boolean(running) || folderBusy}
              onClick={() => void openFolder(active.workspace)}>Reopen task folder</button>
          </div>}
          {terminalCommand?.chatId === active.id && !wrongFolder && <div className="approval terminal-command">
            <span>Run <code>{terminalCommand.text}</code> in Nova’s interactive terminal.</span>
            <div className="approval-actions">
              <button type="button" onClick={() => void navigator.clipboard.writeText(terminalCommand.text)
                .then(() => setNotice('Command copied.'), (error) => setNotice(errorText(error)))}>Copy command</button>
              <button type="button" onClick={launchTerminal} disabled={!workspace || folderBusy}>Open Nova terminal</button>
              <button type="button" className="ghost" aria-label="Dismiss terminal command" onClick={() => setTerminalCommand(null)}>×</button>
            </div>
          </div>}
          {workspace && (offerWorkspaceApproval || needsWorkspaceApproval) && (
            <div className="approval">
              <div className="approval-detail" role="status">
                <strong>Trust this folder for Nova?</strong>
                <p><code>{workspace.root}</code></p>
                <span>Nova can read files and run tools in this folder. It saves your approval for future sessions.</span>
              </div>
              <div className="approval-actions">
                <button type="button" className="approve" disabled={Boolean(running) || folderBusy}
                  onClick={() => {
                    if (!needsWorkspaceApproval || send(lastPrompt.text, active, 'approveWorkspace')) {
                      approveWorkspace(workspace.root)
                    }
                  }}>{needsWorkspaceApproval ? 'Trust folder and retry' : 'Trust folder'}</button>
                {offerWorkspaceApproval && <button type="button" className="deny"
                  onClick={() => setOfferedWorkspace(null)}>Later</button>}
              </div>
            </div>
          )}
          <Composer chatId={active.id} commands={commands} busy={busy} disabled={!workspace || folderBusy || wrongFolder || Boolean(running && !busy)}
            agent={agent} model={taskModel(active, preferences.models[agent])} effort={active.effort}
            recentModels={preferences.recentModels} configuredModels={preferences.models} onSelectModel={selectModel}
            work={busy && running.role ? CREW[running.role].name + (running.iteration && running.iteration > 1
              ? ' · iteration ' + running.iteration : '') + ' · ' + work.toLowerCase() : work}
            waiting={!!interrupt} hasHistory={active.items.length > 0} obscured={settings}
            crew={active.crew} onCrew={(crew) => setChats((previous) => previous.map(
              (chat) => chat.id === active.id ? { ...chat, crew } : chat,
            ))}
            onConfigureCrew={() => openSettings('Agents', 'Agent teams')}
            usage={active.usage} workspace={workspace?.name ?? null}
            onSend={submit} onStop={stopTurn} />
          <TerminalDock key={workspace?.root} dock={dock} height={dockHeight} onDock={setDock} onHeight={setDockHeight}
            workspaceReady={!!workspace} obscured={settings} />
        </main>

        <PanelRail tab={pane} panelOpen={panelOpen} dock={dock}
          onSelect={(next) => { setPane(next); setPanelOpen(true) }}
          onTogglePanel={() => setPanelOpen((open) => !open)}
          onToggleDock={() => setDock((current) => current === 'closed' ? 'open' : 'closed')} />
        <RightPanel workspace={workspace} tab={pane} open={panelOpen} obscured={settings} onSelect={setPane}
          width={panelWidth} onWidth={setPanelWidth} sidebarWidth={sidebarOpen ? railWidth : railCollapsed} />
      </div>
      {settings && <Settings key={workspace?.root} preferences={preferences} update={update}
        initialSection={settingsPage.section} initialTab={settingsPage.tab}
        workspace={workspace} onClose={closeSettings} />}
    </>
  )
}
