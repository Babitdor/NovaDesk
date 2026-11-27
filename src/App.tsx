import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
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
import { TeamPipeline } from './components/TeamPipeline'
import { ViewSwitcher } from './components/ViewSwitcher'
import { appendUser } from './lib/chat'
import {
  CREW, addressedCrew, crewPrompt, parseRetry, systemPrompt, teamOrder,
  type CrewMode, type CrewRole, type CrewSpeaker, type Handoff,
  type StageStatus, type TeamRun, type TeamStage,
} from './lib/crew'
import { clamp, defaultPanelWidth, defaultRailWidth, dockMax, dockMin, panelMax, panelMin, railCollapsed, railMax, railMin } from './lib/layout'
import { withViewTransition } from './lib/motion'
import type { AgentEvent, FinishReason, InterruptReply, WorkspaceInfo } from './lib/events'
import { VIEWS, type Pane, type View } from './lib/panes'
import {
  cancelTurn, errorText, openNovaTerminal, pickWorkspace, prewarmNova, reportCrash, respondApproval,
  setCrashPolicy, setQuickLauncher, setWidget, setWorkspace, startTurn, type TerminalShell,
} from './lib/ipc'
import { MAX_CHATS, createChat, load, routeEvent, save, taskModel, titleFromPrompt, type Chat, type Column } from './lib/sessions'
import { startToolBridge, type ToolHandler } from './lib/toolBridge'
import type { BrowserTarget } from './components/BrowserPane'
import { setDisabledSkills } from './lib/tools'
import { taskTools } from './lib/tasks'
import { advance, dueAfterThread, dueSchedules, promptFor, withinLimit } from './lib/scheduler'
import { speakReply, stopSpeaking } from './lib/speak'
import {
  AGENTS, applyAppearance, clampInterfaceSize, loadPreferences, recommendedAgent, savePreferences,
  type AgentId, type ModelSelection, type Preferences, type ScheduleRun,
} from './lib/settings'
import { allNovaSlashCommands, desktopSlashCommands, novaSlashCommands } from './lib/slash'

const WORKSPACE_KEY = 'nova.desktop.workspace'
type Running = { turnId: string; chatId: string; role?: CrewRole; iteration?: number }

/// The task rows the widget and the quick launcher both report: the one turn
/// that is running, or none. It is a function rather than a hook because two
/// different effects need the same three fields.
function activeRuns(running: Running | null, chats: Chat[]) {
  if (!running) return []
  const chat = chats.find((entry) => entry.id === running.chatId)
  return [{
    chatId: running.chatId,
    title: chat?.title ?? 'Task',
    work: running.role ? CREW[running.role].name : 'Working',
  }]
}
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
  /// True while a @team pipeline (not a plain turn) is driving the live turn.
  const [crewActive, setCrewActive] = useState(false)
  const live = useRef<Running | null>(null)
  const crewRun = useRef<{ cancelled: boolean } | null>(null)
  /// The pipeline's open plan gate, if any: the paused run resolves its loop
  /// through this when the user approves, edits or skips the plan.
  const planGate = useRef<{ runId: string; resolve: (decision: 'approve' | 'skip' | string) => void } | null>(null)
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
  /// Requests from a tool that has to move the UI: the file to show, the page
  /// to open, the shell to start. Each carries a nonce so asking for the same
  /// file twice still counts as a new request.
  const [fileToOpen, setFileToOpen] = useState<{ path: string; nonce: number } | null>(null)
  const [browserTarget, setBrowserTarget] = useState<BrowserTarget | null>(null)
  const browserOpening = useRef(false)
  const [shellToSpawn, setShellToSpawn] = useState<{ shell: TerminalShell; nonce: number } | null>(null)
  /// The tool handler for the current render, and the deferred send a tool
  /// asked for on a task that was not open yet.
  const toolHandler = useRef<ToolHandler>(() => {
    throw new Error('the tool bridge is not ready yet')
  })
  const deferredSend = useRef<{ chatId: string; text: string; settle: (ok: boolean) => void } | null>(null)
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
    // The launcher and the widget wear this theme too. They re-read it whenever
    // they are focused; this reaches the one that is already on screen.
    for (const label of ['launcher', 'widget']) {
      void emitTo(label, 'nova:appearance').catch(() => {})
    }
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

  // The skill switches are honoured by Rust, so it needs the list at startup
  // and not only once Settings has been opened.
  useEffect(() => {
    void setDisabledSkills(preferences.disabledSkills).catch(() => {})
  }, [preferences.disabledSkills])

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
    // The widget's × is a request to stop showing it, not a note that it
    // vanished: turning the preference off is what keeps it closed after a
    // restart, and the effect above then closes the window itself.
    const dismiss = listen('nova:widget-dismiss', () => {
      setPreferences((current) => ({ ...current, widgetEnabled: false }))
    })
    // Tools act on the window through these: opening a file, a page or a shell
    // is a UI decision (which pane, how it is sized), so Rust asks and the page
    // does it rather than growing a second opinion about the layout.
    const openFile = listen<{ path?: string }>('nova:open-file', (event) => {
      const path = String(event.payload?.path ?? '')
      if (!path) return
      setFileToOpen({ path, nonce: Date.now() })
      setView('Files')
    })
    const openTerminal = listen<{ shell?: string }>('nova:open-terminal', (event) => {
      const shell = String(event.payload?.shell ?? 'powershell')
      if (shell !== 'powershell' && shell !== 'cmd' && shell !== 'gitbash') return
      setShellToSpawn({ shell, nonce: Date.now() })
      setDock('open')
    })
    return () => {
      void quick.then((off) => off()).catch(() => {})
      void focusChat.then((off) => off()).catch(() => {})
      void dismiss.then((off) => off()).catch(() => {})
      void openFile.then((off) => off()).catch(() => {})
      void openTerminal.then((off) => off()).catch(() => {})
    }
  }, [chats])

  /// What the widget shows. Sent on every change rather than polled inside the
  /// widget, so the companion window holds no state of its own.
  useEffect(() => {
    if (!preferences.widgetEnabled) return
    void emitTo('widget', 'nova:widget-state', {
      runs: activeRuns(running, chats),
      paused: preferences.pauseSchedules,
    }).catch(() => {})
  }, [preferences.widgetEnabled, preferences.pauseSchedules, running, chats])

  /// Answers the launcher with what to draw: the folder, the model and the task
  /// that is running. Registered with its facts as dependencies, so a palette
  /// raised an hour later gets the current render rather than a copy taken at
  /// mount.
  useEffect(() => {
    const ready = listen('nova:launcher-ready', () => {
      void emitTo('launcher', 'nova:launcher-state', {
        workspace: workspace?.name ?? '',
        model: taskModel(active, preferences.models[agent]) ?? '',
        runs: activeRuns(running, chats),
      }).catch(() => {})
    })
    return () => { void ready.then((off) => off()).catch(() => {}) }
  }, [workspace, active, preferences.models, agent, running, chats])



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

  /// Answers an approval the user has already decided about, in advance.
  /// Returns whether it answered: when it did not, the caller shows the card as
  /// before. Only a tool permission ('tools') and a plan ('all') are covered —
  /// a question needs words, so it is never answered here.
  const autoAnswer = useCallback((event: InterruptRequest, turnId: string, chatId: string, speaker?: CrewSpeaker) => {
    const mode = preferences.autoApprove
    const covered = event.kind === 'tool' ? mode !== 'off' : event.kind === 'plan' && mode === 'all'
    if (!covered || replying.current || live.current?.turnId !== turnId) return false
    replying.current = true
    const reply: InterruptReply = event.kind === 'plan'
      ? { kind: 'plan', approve: true }
      : { kind: 'tool', approve: true }
    void respondApproval(turnId, event.interruptId, reply)
      // Visible, not silent: a decision made without the user has to be in the
      // transcript where they will scroll past it.
      .then(() => setChats((previous) => routeEvent(previous, chatId, {
        type: 'notice', level: 'info',
        message: event.kind === 'plan' ? 'Auto-approved the plan.' : 'Auto-approved a tool permission.',
      }, speaker)))
      .catch((err) => setNotice(errorText(err)))
      .finally(() => {
        if (live.current?.turnId === turnId) replying.current = false
      })
    return true
  }, [preferences.autoApprove])

  const send = useCallback((prompt: string, chat: Chat, mode?: 'scheduled' | 'approveWorkspace') => {
    if (live.current || folderBusy || !workspace || (chat.workspace && chat.workspace !== workspace.root)) return false
    const fresh = mode === 'scheduled'
    const approvingWorkspace = mode === 'approveWorkspace'
    const backend = chat.agent ?? recommendedAgent(preferences, workspace.root)
    const model = taskModel(chat, preferences.models[backend])
    const chatId = chat.id
    const turnId = crypto.randomUUID()
    live.current = { turnId, chatId }
    // A new turn ends whatever the last one was still saying.
    stopSpeaking()
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
        // Reading aloud is a preference, not a step in the turn: a failure to
        // play is a notice, never a failed reply.
        if (event.reason === 'completed') {
          void speakReply(spoken, preferences).then((error) => {
            if (error) setNotice(error)
          })
        }
        if (event.reason === 'failed' && !awaitingWorkspaceApproval) {
          setNotice((message) => message ?? 'The turn failed. Check the task for details.')
        }
      } else if (event.type === 'turnError') {
        awaitingWorkspaceApproval = event.kind === 'workspaceNotApproved'
      } else if (event.type === 'approvalRequired') {
        // Before the queue: an auto-approved request must never render a card,
        // not even for one frame.
        if (!autoAnswer(event, turnId, chatId)) setInterrupts((queue) => [...queue, event])
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
  }, [autoAnswer, folderBusy, preferences, workspace])

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
    planGate.current?.resolve('skip')
    stopSpeaking()
    if (live.current) void cancelTurn(live.current.turnId).catch((error) => setNotice(errorText(error)))
  }
  /// Patch the active chat's team run in place. The run is the pipeline strip's
  /// only input, so every stage transition lands here.
  const patchTeamRun = (chatId: string, patch: (run: TeamRun) => TeamRun) =>
    setChats((previous) => previous.map((chat) => chat.id === chatId && chat.teamRun
      ? { ...chat, teamRun: patch(chat.teamRun) } : chat))

  /// Roles for a team run: the configured pipeline workers, then the reporter.
  /// Nova plans and reports, so it is never dropped.
  const teamRoles = (): readonly CrewRole[] => [
    ...teamOrder.slice(0, -1).filter((role) => role === 'nova' || preferences.team.roles[role] !== false),
    'nova',
  ]

  /// Drives one pipeline stage: ralph's iteration loop, the inactivity watchdog,
  /// stage bookkeeping and the handoff the next stage inherits. Shared by the
  /// first run and the retry-failed-stages path.
  const executeStage = async (options: {
    chatId: string; task: string; mode: CrewMode; roles: readonly CrewRole[]
    index: number; role: CrewRole; final: boolean; notes: Handoff[]; run: { cancelled: boolean }
    history: string
  }) => {
    const { chatId, task, mode, roles, index, role, final, notes, run, history } = options
    const configuration = preferences.crew[role]
    const backend = configuration?.agent ?? 'nova'
    const maxIterations = Math.max(1, preferences.team.ralphIterations)
    const timeoutMs = Math.max(1, preferences.team.stageTimeoutMin) * 60_000
    for (let iteration = 1; !run.cancelled; iteration += 1) {
      const turnId = crypto.randomUUID()
      const speaker = { role, to: roles[index + 1], stage: turnId, scope: mode }
      live.current = { turnId, chatId, role, iteration }
      setRunning(live.current)
      setInterrupts([])
      setReplyPending(false)
      replying.current = false
      patchTeamRun(chatId, (teamRun) => ({ ...teamRun, stages: teamRun.stages.map(
        (stage, at) => at === index
          ? { ...stage, status: 'running', iteration, stageId: turnId, startedAt: stage.startedAt ?? Date.now() }
          : stage,
      ) }))
      let reply = ''
      let timedOut = false
      let watchdog: number | undefined
      const arm = () => {
        clearTimeout(watchdog)
        watchdog = window.setTimeout(() => {
          timedOut = true
          void cancelTurn(turnId).catch(() => {})
        }, timeoutMs)
      }
      const reason: FinishReason = await new Promise<FinishReason>((resolve) => {
        void startTurn({
          turnId, chatId: turnId, backend, model: configuration?.model?.trim() || undefined,
          personality: { role, systemPrompt: configuration?.systemPrompt?.trim() || systemPrompt(role) },
          prompt: crewPrompt(role, task, history, notes, mode === 'team', final),
          trustWorkspace: true, check: final && role !== 'ralph' && role !== 'serena' && role !== 'nora'
            ? preferences.checks[workspace?.root ?? ''] : undefined,
        }, (event) => {
          if (event.type === 'turnStarted' && run.cancelled) void cancelTurn(turnId).catch(() => {})
          if (event.type === 'approvalRequired' && !autoAnswer(event, turnId, chatId, speaker)) {
            setInterrupts((queue) => [...queue, event])
          }
          if (event.type === 'usage') {
            patchTeamRun(chatId, (teamRun) => ({ ...teamRun, usage: {
              input: teamRun.usage.input + event.input, output: teamRun.usage.output + event.output,
            } }))
          }
          if (event.type === 'messageDelta' || (event.type === 'messageFinal' && !event.plain)) {
            const separator = reply && event.type === 'messageFinal' ? '\n\n' : ''
            reply = (reply + separator + event.text).slice(-8000)
          }
          if (event.type !== 'notice' || event.level !== 'info') {
            setChats((previous) => routeEvent(previous, chatId, event, speaker))
          }
          if (event.type !== 'turnFinished') arm()
          if (event.type === 'turnFinished') resolve(event.reason)
        }).catch((error) => {
          setChats((previous) => routeEvent(previous, chatId, { type: 'turnError', message: errorText(error) }, speaker))
          resolve('failed')
        })
      }).finally(() => clearTimeout(watchdog))
      // A user stop ends the run; the backend reporting a stage as cancelled on
      // its own must not — it only loses that stage's work.
      if (run.cancelled) break
      if (timedOut) {
        setChats((previous) => routeEvent(previous, chatId, {
          type: 'turnError', message: `${CREW[role].name} was silent for ${preferences.team.stageTimeoutMin} minutes — the stage was cancelled.`,
        }, speaker))
      } else if (reason === 'cancelled') {
        setNotice(CREW[role].name + ' was interrupted before finishing — the team continues without that stage.')
      }
      const parsed = parseRetry(reply)
      const retry = role === 'ralph' && reason === 'completed' && parsed.retry
      const status: StageStatus = reason === 'completed' && !timedOut ? 'done' : 'failed'
      notes.push({ from: role, text: status === 'done'
        ? parsed.clean || 'No written handoff was returned.'
        : 'This stage failed. See the transcript for details and do not treat this work as verified.' })
      patchTeamRun(chatId, (teamRun) => ({
        ...teamRun,
        stages: teamRun.stages.map((stage, at) => at === index
          ? { ...stage, status: retry ? 'running' : status, iteration, finishedAt: retry ? undefined : Date.now() }
          : stage),
        handoffs: notes.map(({ from, text }) => ({ from, text: text.slice(0, 2000) })),
      }))
      if (!retry || iteration >= maxIterations) {
        if (retry) setNotice(`${CREW[role].name} reached the ${maxIterations}-iteration limit. A check still needs attention.`)
        break
      }
    }
  }

  const sendCrew = (task: string, mode: CrewMode) => {
    if (live.current || !workspace || folderBusy || wrongFolder) return false
    if (!preferences.trustedWorkspaces.includes(workspace.root)) {
      setOfferedWorkspace(workspace.root)
      return false
    }
    const chatId = active.id
    const roles = mode === 'team' ? teamRoles() : [mode]
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
    setCrewActive(true)
    setNotice(null)
    const stages: TeamStage[] = roles.map((role) => ({ role, status: 'pending', iteration: 1 }))
    setChats((previous) => previous.map((chat) => chat.id === chatId ? {
      ...chat, workspace: workspace.root, state: 'running', items: appendUser(chat.items, task, mode),
      title: chat.title === 'New task' ? titleFromPrompt(task) : chat.title,
      teamRun: { id: crypto.randomUUID(), task, stages, startedAt: Date.now(),
        usage: { input: 0, output: 0 }, handoffs: [], awaitingPlan: null },
    } : chat))

    void (async () => {
      for (const [index, role] of roles.entries()) {
        const final = index === roles.length - 1
        await executeStage({ chatId, task, mode, roles, index, role, final, notes, run, history })
        if (run.cancelled) break
        // The plan gate: Nova's plan pauses the pipeline until the user
        // approves, rewrites or skips it — only when the setting asks for it.
        if (mode === 'team' && preferences.team.planApproval && index === 0 && notes.length) {
          patchTeamRun(chatId, (teamRun) => ({ ...teamRun, awaitingPlan: index + 1 }))
          const decision = await new Promise<'approve' | 'skip' | string>((resolve) => {
            planGate.current = { runId: '', resolve }
          })
          planGate.current = null
          patchTeamRun(chatId, (teamRun) => ({ ...teamRun, awaitingPlan: null }))
          if (typeof decision === 'string' && decision.trim()) {
            notes.push({ from: 'user', text: 'User steering after reviewing the plan: ' + decision.trim().slice(0, 2000) })
          }
        }
        if (run.cancelled) break
      }
    })().finally(() => {
      crewRun.current = null
      planGate.current = null
      setCrewActive(false)
      live.current = null
      setRunning(null)
      setInterrupts([])
      setReplyPending(false)
      replying.current = false
      setChats((previous) => previous.map((chat) => chat.id === chatId ? {
        ...chat, state: 'idle',
        teamRun: chat.teamRun ? {
          ...chat.teamRun, finishedAt: Date.now(),
          stages: chat.teamRun.stages.map((stage) => stage.status === 'running'
            ? { ...stage, status: 'failed' as StageStatus, finishedAt: Date.now() } : stage),
        } : undefined,
      } : chat))
    }).catch((error) => setNotice(errorText(error)))
    return true
  }
  /// Re-runs only the failed stages of the last team run, replaying the recorded
  /// handoffs so the retry picks up where the run stopped.
  const retryTeamStages = () => {
    const run = active.teamRun
    if (!run || live.current || !workspace || folderBusy || wrongFolder) return
    if (!preferences.trustedWorkspaces.includes(workspace.root)) {
      setOfferedWorkspace(workspace.root)
      return
    }
    const failed = run.stages
      .map((stage, index) => ({ stage, index }))
      .filter(({ stage }) => stage.status === 'failed')
    if (!failed.length) return
    const chatId = active.id
    const cancelled = { cancelled: false }
    crewRun.current = cancelled
    setCrewActive(true)
    setNotice(null)
    const notes: Handoff[] = run.handoffs.map(({ from, text }) => ({ from, text }))
    const history = active.items.flatMap((item) => {
      if (item.kind === 'user' && item.recipient === 'team') return ['User: ' + item.text.slice(0, 2000)]
      if (item.kind === 'assistant' && !item.provisional && item.speaker &&
        (item.speaker.scope ?? 'team') === 'team') {
        return [CREW[item.speaker.role].name + ': ' + item.text.slice(0, 2000)]
      }
      return []
    }).slice(-8).join('\n\n')
    patchTeamRun(chatId, (teamRun) => ({
      ...teamRun, finishedAt: undefined, handoffs: notes,
      stages: teamRun.stages.map((stage, index) =>
        failed.some(({ index: at }) => at === index)
          ? { ...stage, status: 'pending', startedAt: undefined, finishedAt: undefined } : stage),
    }))
    void (async () => {
      for (const { index } of failed) {
        const role = run.stages[index].role
        await executeStage({
          chatId, task: run.task, mode: 'team', roles: run.stages.map(({ role }) => role),
          index, role, final: index === run.stages.length - 1, notes, run: cancelled, history,
        })
        if (cancelled.cancelled) break
      }
    })().finally(() => {
      crewRun.current = null
      setCrewActive(false)
      live.current = null
      setRunning(null)
      setInterrupts([])
      setReplyPending(false)
      replying.current = false
      setChats((previous) => previous.map((chat) => chat.id === chatId ? {
        ...chat, state: 'idle',
        teamRun: chat.teamRun ? { ...chat.teamRun, finishedAt: Date.now() } : undefined,
      } : chat))
    }).catch((error) => setNotice(errorText(error)))
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

  /// Runs a prompt in one task, whether or not it is open. A tool that asked for
  /// another task switches to it and settles when the composer's own path has
  /// run, so the answer it returns is the real outcome rather than a guess.
  const startIn = (chatId: string, text: string) => new Promise<boolean>((resolve) => {
    if (live.current || deferredSend.current || !chats.some(({ id }) => id === chatId)) {
      resolve(false)
      return
    }
    if (chatId === active.id) {
      resolve(submitRef.current(text))
      return
    }
    deferredSend.current = { chatId, text, settle: resolve }
    setActiveId(chatId)
  })

  useEffect(() => {
    const request = deferredSend.current
    if (!request) return
    deferredSend.current = null
    request.settle(request.chatId === active.id && submitRef.current(request.text))
  }, [active.id])

  /// The handler the tool bridge calls. Kept in a ref updated after every
  /// render so the listener registered once still answers from current state.
  useLayoutEffect(() => {
    const tasks = taskTools({
      chats: () => chats,
      activeId: () => active.id,
      liveChat: () => live.current?.chatId ?? null,
      open: (id) => {
        setActiveId(id)
        setView('Chat')
        void getCurrentWindow().setFocus().catch(() => {})
      },
      create: (title, agent) => {
        const chat: Chat = { ...createChat(), title, ...(agent ? { agent } : {}) }
        addTasks([chat])
        return chat
      },
      update: (id, patch) => setChats((previous) => previous.map(
        (chat) => chat.id === id ? { ...chat, ...patch } : chat)),
      archive: (id) => {
        // Same rule as the rail's ×: deleting under a live turn would leave the
        // stream with nowhere to land.
        if (live.current?.chatId === id) return 'Stop the running turn before deleting this task.'
        const index = chats.findIndex((chat) => chat.id === id)
        const remaining = chats.filter((chat) => chat.id !== id)
        if (!remaining.length) remaining.push(createChat())
        setChats(remaining)
        if (activeId === id) setActiveId(remaining[Math.min(index, remaining.length - 1)].id)
        return null
      },
      start: startIn,
    })
    toolHandler.current = async (name, args) => {
      if (name === 'deliver_queued_message') {
        if (typeof args.chatId !== 'string' || typeof args.text !== 'string') throw new Error('Invalid queued message')
        if (await startIn(args.chatId, args.text)) return true
        const message = 'A queued message could not start and will stay in the queue.'
        setNotice(message)
        throw new Error(message)
      }
      if (name !== 'open_browser') return tasks(name, args)
      if (typeof args.url !== 'string') throw new Error('url is required')
      if (settings || browserOpening.current) throw new Error('The browser pane is busy. Try again.')
      browserOpening.current = true
      try {
        const url = args.url
        const opened = await new Promise<string>((settle) => {
          setBrowserTarget({ url, settle })
          setView('Preview')
        })
        return { opened }
      } finally {
        browserOpening.current = false
        setBrowserTarget(null)
      }
    }
  })

  /// One listener for the whole session: Rust emits nova:tool-call and waits on
  /// a one-shot for the answer, so a second registration would answer twice.
  useEffect(() => {
    let dispose: (() => void) | null = null
    let cancelled = false
    void startToolBridge(() => (name, args) => toolHandler.current(name, args))
      .then((off) => { if (cancelled) off(); else dispose = off })
      .catch(() => {})
    return () => {
      cancelled = true
      if (dispose) dispose()
    }
  }, [])
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
          {active.teamRun && <TeamPipeline run={active.teamRun} busy={busy && crewActive}
            onDecision={(decision) => planGate.current?.resolve(decision)}
            onRetry={retryTeamStages} />}
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
            workspaceReady={!!workspace} obscured={settings} spawn={shellToSpawn} />
        </main>

        <RightPanel workspace={workspace} tab={pane} open={panelOpen} obscured={settings}
          width={panelWidth} onWidth={setPanelWidth} sidebarWidth={sidebarOpen ? railWidth : railCollapsed}
          chats={chats} activeId={active.id} preferences={preferences} onOpenChat={openChat}
          onMove={moveChat} onUpdatePreferences={update} onOpenFolder={(path) => void openFolder(path)}
          fileToOpen={fileToOpen} browserTarget={browserTarget} />
      </div>
      {settings && <Settings key={workspace?.root} preferences={preferences} update={update}
        initialSection={settingsPage.section} initialTab={settingsPage.tab}
        workspace={workspace} onClose={closeSettings} />}
    </>
  )
}
