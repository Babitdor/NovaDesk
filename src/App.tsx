import { useCallback, useEffect, useRef, useState } from 'react'
import { emitTo, listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { ChatLog } from './components/ChatLog'
import { Composer } from './components/Composer'
import { Interrupt } from './components/Interrupt'
import { RightPanel } from './components/RightPanel'
import { Settings, type Section } from './components/Settings'
import { Sidebar } from './components/Sidebar'
import { Sky } from './components/Sky'
import { TerminalDock, type Dock } from './components/TerminalDock'
import { ViewSwitcher } from './components/ViewSwitcher'
import { appendUser } from './lib/chat'
import { CREW, addressedCrew, crewPrompt, ralphRetry, systemPrompt, teamOrder, type CrewMode, type CrewRole, type Handoff } from './lib/crew'
import { clamp, defaultPanelWidth, defaultRailWidth, dockMax, dockMin, panelMax, panelMin, railCollapsed, railMax, railMin } from './lib/layout'
import { withViewTransition } from './lib/motion'
import type { AgentEvent, FinishReason, InterruptReply, WorkspaceInfo } from './lib/events'
import { VIEWS, type Pane, type View } from './lib/panes'
import {
  cancelTurn, errorText, openNovaTerminal, pickWorkspace, prewarmNova, reportCrash, respondApproval,
  setCrashPolicy, setQuickLauncher, setWidget, setWorkspace, startTurn,
} from './lib/ipc'
import { MAX_CHATS, createChat, load, routeEvent, save, taskModel, titleFromPrompt, type Chat, type Column } from './lib/sessions'
import { advance, dueAfterThread, dueSchedules, promptFor, withinLimit } from './lib/scheduler'
import {
  AGENTS, applyAppearance, clampInterfaceSize, loadPreferences, recommendedAgent, savePreferences,
  type AgentId, type ModelSelection, type Preferences, type ScheduleRun,
} from './lib/settings'
import { allNovaSlashCommands, desktopSlashCommands, novaSlashCommands } from './lib/slash'

const WORKSPACE_KEY = 'nova.desktop.workspace'
type Running = { turnId: string; chatId: string; role?: CrewRole; iteration?: number }
type InterruptRequest = Extract<AgentEvent, { type: 'approvalRequired' }>
/// A start requested by an agent rather than by the user. `needsApproval` is the
/// "Ask before agents message other threads" gate; nothing in the queue runs
/// while it is true.
type BridgeRequest = { id: string; prompt: string; agent?: AgentId; needsApproval: boolean }
type BridgeEvent = Extract<AgentEvent, { type: 'wakeupRequested' | 'threadMessage' }>

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
  const [settingsPage, setSettingsPage] = useState<{ section: Section; tab: string }>({ section: 'General', tab: 'Harnesses' })
  const [bridgeQueue, setBridgeQueue] = useState<BridgeRequest[]>([])
  const [terminalCommand, setTerminalCommand] = useState<{ chatId: string; text: string } | null>(null)
  const [running, setRunning] = useState<Running | null>(null)
  const live = useRef<Running | null>(null)
  const crewRun = useRef<{ cancelled: boolean } | null>(null)
  const replying = useRef(false)
  /// When the last turn finished: the gate for an "after a thread finishes" run.
  const lastFinish = useRef(0)
  /// Latest `submit` and bridge handler, so the event listeners registered once
  /// at mount always call the current render's closures.
  const submitRef = useRef<(text: string) => boolean>(() => false)
  const bridgeRef = useRef<(event: BridgeEvent) => void>(() => {})
  const [notice, setNotice] = useState<string | null>(null)
  const [offeredWorkspace, setOfferedWorkspace] = useState<string | null>(null)
  const [view, setView] = useState<View>('Chat')
  const [dock, setDock] = useState<Dock>('closed')
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [railWidth, setRailWidth] = useState(defaultRailWidth)
  const [panelWidth, setPanelWidth] = useState(defaultPanelWidth)
  const [dockHeight, setDockHeight] = useState<number | null>(null)
  const [interrupts, setInterrupts] = useState<InterruptRequest[]>([])
  const [replyPending, setReplyPending] = useState(false)
  // Chat is the transcript with no panel, so one state answers both questions the
  // shell used to keep in two: which pane is showing, and whether it is open.
  const panelOpen = view !== 'Chat'
  const pane: Pane = view === 'Chat' ? 'Files' : view
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

  /// One activity entry. The log is a convenience, not a journal: fifty rows and
  /// nothing older, so a long-running schedule cannot grow storage forever.
  const record = useCallback((scheduleId: string, status: ScheduleRun['status'], detail: string) => {
    setPreferences((current) => ({
      ...current,
      scheduleActivity: [{ id: crypto.randomUUID(), scheduleId, at: Date.now(), status,
        detail: detail.trim().slice(0, 160) || 'Scheduled run' }, ...current.scheduleActivity].slice(0, 50),
    }))
  }, [])

  // The three native settings are written on load as well as on change: the
  // recovery policy has to be on disk before a crash can read it, and a shortcut
  // has to be registered again after a restart.
  useEffect(() => {
    void setCrashPolicy(preferences.restartAfterCrash, preferences.crashReports).catch(() => {})
  }, [preferences.restartAfterCrash, preferences.crashReports])

  useEffect(() => {
    void setQuickLauncher(preferences.quickLauncher, preferences.quickLauncherShortcut).catch((err) => {
      // Another application may already own the chord: say so and stop claiming the
      // launcher is on, rather than leaving a toggle that does nothing.
      setNotice('The global shortcut could not be registered: ' + errorText(err))
      setPreferences((current) => ({ ...current, quickLauncher: false }))
    })
  }, [preferences.quickLauncher, preferences.quickLauncherShortcut])

  useEffect(() => {
    void setWidget(preferences.widgetEnabled, preferences.widgetPinned).catch(() => {})
  }, [preferences.widgetEnabled, preferences.widgetPinned])

  /// Page errors are reported once per ten seconds: a render loop would
  /// otherwise fill the crash folder with the same stack.
  useEffect(() => {
    if (!preferences.crashReports) return
    let last = 0
    const report = (message: string, location: string) => {
      const now = Date.now()
      if (now - last < 10_000) return
      last = now
      void reportCrash(message.slice(0, 4000) || 'Unknown error', location.slice(0, 400)).catch(() => {})
    }
    const onError = (event: ErrorEvent) => report(event.message || 'Uncaught error',
      event.filename + ':' + event.lineno + ':' + event.colno)
    const onRejection = (event: PromiseRejectionEvent) => {
      const reason: unknown = event.reason
      const text = typeof reason === 'string' ? reason
        : (reason as { message?: string } | null)?.message ?? 'Unhandled rejection'
      report(text, typeof reason === 'object' && reason
        ? ((reason as { stack?: string }).stack ?? '').slice(0, 400) : '')
    }
    window.addEventListener('error', onError)
    window.addEventListener('unhandledrejection', onRejection)
    return () => {
      window.removeEventListener('error', onError)
      window.removeEventListener('unhandledrejection', onRejection)
    }
  }, [preferences.crashReports])

  /// The two auxiliary windows talk back through events only — the capability
  /// gives them no commands — so these listeners are the whole receiving side.
  useEffect(() => {
    const quick = listen<{ prompt?: string }>('nova:quick-task', (event) => {
      const prompt = String(event.payload?.prompt ?? '').slice(0, 12000)
      if (!prompt.trim()) return
      void getCurrentWindow().setFocus().catch(() => {})
      submitRef.current(prompt)
    })
    const focusChat = listen<{ chatId?: string }>('nova:focus-chat', (event) => {
      const chatId = String(event.payload?.chatId ?? '')
      if (chats.some((chat) => chat.id === chatId)) {
        withViewTransition(() => { setActiveId(chatId); setView('Chat') })
      }
      void getCurrentWindow().setFocus().catch(() => {})
    })
    return () => {
      void quick.then((off) => off()).catch(() => {})
      void focusChat.then((off) => off()).catch(() => {})
    }
  }, [chats])

  /// What the widget shows. Sent on every change rather than polled inside the
  /// widget, so the companion window holds no state of its own.
  useEffect(() => {
    if (!preferences.widgetEnabled) return
    const chat = chats.find((entry) => entry.id === running?.chatId)
    const runs = running
      ? [{ chatId: running.chatId,
        title: chat?.title ?? 'Task',
        work: running.role ? CREW[running.role].name : 'Working' }]
      : []
    void emitTo('widget', 'nova:widget-state', { runs, paused: preferences.pauseSchedules }).catch(() => {})
  }, [preferences.widgetEnabled, preferences.pauseSchedules, running, chats])



  useEffect(() => {
    if (!restoredPath) return
    void setWorkspace(restoredPath).then(setFolder)
      .catch((err) => setNotice(errorText(err))).finally(() => setFolderBusy(false))
  }, [restoredPath])

  /// A trusted folder gets a warm Nova session while the user is still typing,
  /// so the first turn streams instead of booting a CLI process.
  const workspaceRoot = workspace?.root
  const workspaceTrusted = Boolean(workspaceRoot && preferences.trustedWorkspaces.includes(workspaceRoot))
  useEffect(() => {
    if (workspaceRoot && workspaceTrusted) void prewarmNova().catch(() => {})
  }, [workspaceRoot, workspaceTrusted])

  useEffect(() => {
    const timer = setTimeout(() => save(chats), 400)
    const flush = () => save(chats)
    window.addEventListener('pagehide', flush)
    return () => { clearTimeout(timer); window.removeEventListener('pagehide', flush) }
  }, [chats])

  /// One entry point for every way a view changes — the switcher, the number row
  /// and a card that opens its task — so the crossfade and the width rule cannot
  /// drift apart between them.
  const selectView = useCallback((next: View) => withViewTransition(() => {
    setView(next)
    // Three columns of cards want the width: a narrower panel is widened rather
    // than left with clipped columns, and a wider one is left where it was.
    if (next === 'Board') {
      setPanelWidth((width) => Math.max(width, Math.min(panelMax(sidebarOpen ? railWidth : railCollapsed), 560)))
    }
  }), [railWidth, sidebarOpen])

  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key === ',') {
        event.preventDefault()
        setSettings((open) => !open)
      } else if (event.ctrlKey && !event.altKey && event.code === 'Backquote') {
        event.preventDefault()
        setSettings(false)
        setDock((current) => current === 'closed' ? 'open' : 'closed')
      } else if (event.ctrlKey && !event.altKey && /^[1-5]$/.test(event.key)) {
        event.preventDefault()
        setSettings(false)
        selectView(VIEWS[Number(event.key) - 1])
      } else if ((event.ctrlKey || event.metaKey) && !event.altKey
          && (event.key === '=' || event.key === '+' || event.key === '-' || event.key === '0')) {
        // Same clamp the General pane uses, so the keyboard and the buttons can
        // never disagree about the range.
        event.preventDefault()
        const delta = event.key === '0' ? 0 : event.key === '-' ? -10 : 10
        setPreferences((current) => ({
          ...current,
          interfaceSize: delta === 0 ? 100 : clampInterfaceSize(current.interfaceSize + delta),
        }))
      } else if (event.key === 'Escape') {
        setSettings(false)
      }
    }
    window.addEventListener('keydown', shortcut)
    return () => window.removeEventListener('keydown', shortcut)
  }, [selectView])

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
      // Projects is a recency list, newest first, capped: removing an entry
      // forgets the folder and nothing else.
      update({ projects: [{ root: picked.root, name: picked.name, openedAt: Date.now() },
        ...preferences.projects.filter((project) => project.root !== picked.root)].slice(0, 12) })
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
      lastFinish.current = Date.now()
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
      } else if (event.type === 'wakeupRequested' || event.type === 'threadMessage') {
        bridgeRef.current(event)
      }
      if (event.type !== 'notice') setChats((previous) => routeEvent(previous, chatId, event))
    }).catch((err) => {
      finish()
      setChats((previous) => routeEvent(previous, chatId, { type: 'turnError', message: errorText(err) }))
    })
    return true
  }, [folderBusy, preferences, workspace])

  /// The schedule engine owns the decisions; this effect only starts turns and
  /// records what happened. The entry is advanced *before* the turn starts, so a
  /// slow backend can never let the same run fire twice.
  useEffect(() => {
    if (!workspace) return
    const root = workspace.root
    const tick = () => {
      if (live.current || preferences.pauseSchedules) return
      const now = Date.now()
      const due = dueSchedules(preferences.schedules, root, now)
      const afterThread = dueAfterThread(preferences.schedules, root, lastFinish.current)
      const schedule = due[0] ?? afterThread[0]
      if (!schedule) return
      const prompt = promptFor(schedule)
      if (!prompt.trim()) {
        update({ schedules: preferences.schedules.filter((entry) => entry.id !== schedule.id) })
        record(schedule.id, 'failed', 'Empty prompt, so the schedule was removed.')
        return
      }
      if (schedule.origin !== 'user' && !withinLimit(0, bridgeQueue.length, preferences.agentLimit)) {
        record(schedule.id, 'skipped', 'Agent limit reached: ' + prompt)
        return
      }
      const next = advance(schedule, now)
      update({ schedules: next
        ? preferences.schedules.map((entry) => entry.id === schedule.id ? { ...entry, ...next } : entry)
        : preferences.schedules.filter((entry) => entry.id !== schedule.id) })
      const chat = { ...createChat(), agent: schedule.agent, workspace: root }
      if (send(prompt, chat, 'scheduled')) record(schedule.id, 'started', prompt)
      else record(schedule.id, 'failed', 'The task could not start: ' + prompt)
    }
    const timer = setInterval(tick, 5000)
    return () => clearInterval(timer)
  }, [workspace, preferences, send, update, record, bridgeQueue.length])

  /// Cross-thread starts approved by the user run here, one at a time, always
  /// behind the task the user is already in.
  useEffect(() => {
    if (running || folderBusy || !workspace) return
    const next = bridgeQueue.find((request) => !request.needsApproval)
    if (!next) return
    const chat = { ...createChat(), agent: next.agent ?? recommendedAgent(preferences, workspace.root),
      workspace: workspace.root }
    // Starting the run is the external action; the queue update follows it, so a
    // start that fails is recorded instead of looking like a lost request.
    void Promise.resolve().then(() => {
      const started = send(next.prompt, chat, 'scheduled')
      record('bridge', started ? 'started' : 'failed',
        (started ? '' : 'Could not start: ') + next.prompt)
      setBridgeQueue((queue) => queue.filter(({ id }) => id !== next.id))
    })
  }, [running, folderBusy, workspace, bridgeQueue, send, record, preferences])

  /// The receiving end of the request file: the only non-user way a turn starts.
  /// Assigned from an effect so the ref never changes during render; without a
  /// dependency array it always points at the newest render's closures.
  useEffect(() => {
    bridgeRef.current = (event) => {
      const prompt = (event.type === 'wakeupRequested' ? event.prompt : event.text).trim().slice(0, 12000)
      if (!prompt) return
      if (!preferences.agentsMaySchedule) {
        setNotice('An agent asked to start another task, but agent scheduling is off.')
        record('bridge', 'denied', prompt)
        return
      }
      if (!withinLimit(running ? 1 : 0, bridgeQueue.length, preferences.agentLimit)) {
        setNotice('The agent limit for this task is reached, so the request was dropped.')
        record('bridge', 'skipped', prompt)
        return
      }
      const agent = event.type === 'wakeupRequested' && event.agent && event.agent in AGENTS
        ? event.agent as AgentId : undefined
      const request: BridgeRequest = { id: crypto.randomUUID(), prompt, agent,
        needsApproval: preferences.askBeforeCrossThread }
      setBridgeQueue((queue) => [...queue, request].slice(0, 20))
      record('bridge', request.needsApproval ? 'queued' : 'approved',
        (request.needsApproval ? 'Approval needed: ' : 'Queued: ') + prompt)
    }
  })

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

  const openSettings = (section: Section = 'General', tab = 'Harnesses') => {
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
          // A user stop ends the run; the backend reporting a stage as cancelled
          // on its own must not — it only loses that stage's work.
          if (run.cancelled) break
          if (reason === 'cancelled') {
            setNotice(CREW[role].name + ' was interrupted before finishing — the team continues without that stage.')
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

  // The listeners registered at mount call through these refs, so a launcher
  // keystroke always uses the current render's state. Assigned from an effect
  // because a ref must not change during render.
  useEffect(() => { submitRef.current = submit })
  const bridge = bridgeQueue.find((request) => request.needsApproval)
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

  /// A card opens its task, which means the transcript: the board is a way into a
  /// task, not a place to read one.
  const openChat = (id: string) => withViewTransition(() => {
    setActiveId(id)
    setView('Chat')
  })

  const moveChat = (id: string, column: Column) => setChats((previous) => previous.map(
    (chat) => chat.id === id ? { ...chat, column } : chat))

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

        <div className="viewbar">
          <ViewSwitcher view={view} dock={dock} onSelect={selectView}
            onToggleDock={() => setDock((current) => current === 'closed' ? 'open' : 'closed')} />
        </div>

        <main className="main">
          {preferences.wallpaper && <Sky still={preferences.still} />}
          <ChatLog key={active.id} items={active.items} busy={busy}
            markdown={preferences.markdown} backend={backendName} workspaceReady={!!workspace}
            compact={preferences.compactConversation} expandTools={preferences.expandToolActivity} />
          {interrupt && <Interrupt key={interrupt.interruptId} request={interrupt} pending={replyPending}
            onReply={(reply) => void answer(reply)} />}
          {bridge && <div className="approval bridge-approval">
            <div className="approval-detail" role="status">
              <strong>Start another task?</strong>
              <p><code>{bridge.prompt.slice(0, 240)}</code></p>
              <span>An agent asked Nova to start this task. Nothing runs until you approve; a denial is
                written to the Schedules activity log.</span>
            </div>
            <div className="approval-actions">
              <button type="button" className="approve" onClick={() => {
                setBridgeQueue((queue) => queue.map((entry) => entry.id === bridge.id
                  ? { ...entry, needsApproval: false } : entry))
                record('bridge', 'approved', bridge.prompt)
              }}>Approve</button>
              <button type="button" className="deny" onClick={() => {
                setBridgeQueue((queue) => queue.filter(({ id }) => id !== bridge.id))
                record('bridge', 'denied', bridge.prompt)
              }}>Deny</button>
            </div>
          </div>}
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
            voice={preferences.voice} onOpenVoice={() => openSettings('Voice')}
            usage={active.usage} workspace={workspace?.name ?? null}
            keepDrafts={preferences.keepDrafts} onSend={submit} onStop={stopTurn} />
          <TerminalDock key={workspace?.root} dock={dock} height={dockHeight} onDock={setDock} onHeight={setDockHeight}
            workspaceReady={!!workspace} obscured={settings} />
        </main>

        <RightPanel workspace={workspace} tab={pane} open={panelOpen} obscured={settings}
          width={panelWidth} onWidth={setPanelWidth} sidebarWidth={sidebarOpen ? railWidth : railCollapsed}
          chats={chats} activeId={active.id} preferences={preferences} onOpenChat={openChat}
          onMove={moveChat} onUpdatePreferences={update} onOpenFolder={(path) => void openFolder(path)} />
      </div>
      {settings && <Settings key={workspace?.root} preferences={preferences} update={update}
        initialSection={settingsPage.section} initialTab={settingsPage.tab}
        workspace={workspace} onClose={closeSettings} />}
    </>
  )
}
