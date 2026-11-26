import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'

import type { WorkspaceInfo } from '../lib/events'
import {
  cloneParentPick, cloneRepository, githubAccount, githubActivity, githubRepos,
  parseFullName, relativeTime,
  type Activity, type GithubAccount, type Repo as RemoteRepo,
} from '../lib/github'
import { git } from '../lib/git'
import { errorText, killProcess, openLink } from '../lib/ipc'
import type { Preferences } from '../lib/settings'
import { ChangesPane } from './ChangesPane'

type Commit = { hash: string; subject: string; when: string }

type Repo = {
  branch: string
  origin: string
  ahead: number
  behind: number
  changed: number
  commits: Commit[]
}

type Read =
  | { kind: 'repo'; repo: Repo }
  | { kind: 'not-a-repo' }
  | { kind: 'failed'; message: string }

/// An `origin` a browser can open: the ssh form is rewritten to https, which is
/// the same repository on the same host. A local path or an `ssh://` URL with a
/// port has no page to open, so the button says so instead of guessing at one.
function webUrl(origin: string): string | null {
  const trimmed = origin.trim().replace(/\.git$/, '')
  const scp = /^git@([^:]+):(.+)$/.exec(trimmed)
  if (scp) return `https://${scp[1]}/${scp[2]}`
  const ssh = /^ssh:\/\/git@([^/]+)\/(.+)$/.exec(trimmed)
  if (ssh) return `https://${ssh[1]}/${ssh[2]}`
  return /^https?:\/\//i.test(trimmed) ? trimmed : null
}

/// Everything the header and the commit list claim, read from the workspace with
/// no network and no auth: `git` already knows it locally, and this pane is a
/// view of the folder rather than a client for the host.
async function readRepo(): Promise<Read> {
  const inside = await git('rev-parse', '--is-inside-work-tree')
  if (!inside.ok) return { kind: 'not-a-repo' }

  // Sequential, like every other read here: one runner, one command at a time,
  // so a slow repository cannot stack four processes behind one click.
  const branch = await git('rev-parse', '--abbrev-ref', 'HEAD')
  const origin = await git('remote', 'get-url', 'origin')
  const status = await git('status', '--porcelain=v1', '--branch')
  const log = await git('log', '-5', '--pretty=%h%x09%s%x09%ar')

  if (!branch.ok || !status.ok) {
    return { kind: 'failed', message: (branch.err || status.err).trim() || 'git could not read this folder.' }
  }

  // The branch line of `--branch` is the only place ahead/behind is stated, so
  // it is parsed rather than counted, and the count is what is left after it.
  const lines = status.out.split('\n').filter(Boolean)
  const head = lines[0]?.startsWith('##') ? lines[0].slice(2) : ''
  const number = (label: string) => Number(new RegExp(`${label} (\\d+)`).exec(head)?.[1] ?? 0)

  const commits: Commit[] = []
  for (const line of log.ok ? log.out.split('\n') : []) {
    const [hash, subject, when] = line.split('\t')
    if (hash && subject && when) commits.push({ hash, subject, when })
  }

  return {
    kind: 'repo',
    repo: {
      branch: branch.out.trim() === 'HEAD' ? 'detached' : branch.out.trim(),
      origin: origin.ok ? origin.out.trim() : '',
      ahead: number('ahead'),
      behind: number('behind'),
      changed: lines.filter((line) => !line.startsWith('##')).length,
      commits,
    },
  }
}

/// The repository as far as the folder can say: branch, how far it has drifted
/// from its upstream, how many files are dirty, the last few commits, then the
/// working-tree diff itself. Changes retired into this pane — a diff belongs
/// next to the branch it will be committed to.
function LocalRepoView({ workspace }: { workspace: WorkspaceInfo | null }) {
  const [read, setRead] = useState<Read | null>(null)
  const [busy, setBusy] = useState(false)
  /// Bumped on every refresh so the diff re-reads with the header. It is all the
  /// pane needs to keep one button for the whole view.
  const [tick, setTick] = useState(0)
  const [failure, setFailure] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    if (!workspace) return
    setBusy(true)
    setRead(await readRepo())
    setTick((value) => value + 1)
    setBusy(false)
  }, [workspace])

  // The pane is a view of a folder that changes while it is open, so the read
  // belongs to the effect; the diff below re-reads on its own tick.
  useEffect(() => {
    if (!workspace) return
    let alive = true
    void readRepo().then((next) => {
      if (alive) setRead(next)
    })
    return () => {
      alive = false
    }
  }, [workspace])

  const repo = read?.kind === 'repo' ? read.repo : null
  const url = repo ? webUrl(repo.origin) : null
  const drift = repo && (repo.ahead > 0 || repo.behind > 0) ? `\u2191${repo.ahead} \u2193${repo.behind}` : ''
  const state = !workspace
    ? 'no folder open'
    : read === null || busy
      ? 'reading\u2026'
      : read.kind === 'failed'
        ? 'git failed'
        : read.kind === 'not-a-repo'
          ? 'no repository'
          : [repo?.branch, drift, `${repo?.changed ?? 0} changed`].filter(Boolean).join(' \u00b7 ')

  return (
    <section className="github">
      <header className="pane-head">
        <span className="github-state">{state}</span>
        <span className="github-actions">
          <button type="button" className="ghost" onClick={() => void refresh()} disabled={busy || !workspace}>
            Refresh repo
          </button>
          <button
            type="button"
            className="ghost"
            disabled={!url}
            title={url ?? 'Add a remote named origin that points at GitHub.'}
            onClick={() => {
              if (!url) return
              setFailure(null)
              void openLink(url).catch((err) => setFailure(errorText(err)))
            }}
          >
            {url ? 'Open on GitHub' : 'No GitHub remote'}
          </button>
        </span>
      </header>

      {read?.kind === 'failed' && <p className="panel-error">{read.message}</p>}
      {failure && <p className="panel-error">{failure}</p>}

      {repo && repo.commits.length > 0 && (
        <ol className="commit-list">
          {repo.commits.map((commit) => (
            <li key={commit.hash} className="commit-row">
              <code className="commit-hash">{commit.hash}</code>
              <span className="commit-subject">{commit.subject}</span>
              <span className="commit-when">{commit.when}</span>
            </li>
          ))}
        </ol>
      )}
      {repo && repo.commits.length === 0 && <p className="hint">No commits yet.</p>}

      <ChangesPane workspace={workspace} refreshKey={tick} />
    </section>
  )
}

type Props = {
  workspace: WorkspaceInfo | null
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
  onOpenFolder: (path: string) => void
}

type CloneState = { fullName: string; id: string; text: string; error: string | null }

/// The newest 50 clones: this is a shortcut to folders already opened, not an
/// index of the disk, so an old entry is worth less than staying bounded.
function rememberClone(map: Record<string, string>, full: string, path: string) {
  const next = { ...map, [full]: path }
  const keys = Object.keys(next)
  if (keys.length <= 50) return next
  return Object.fromEntries(keys.slice(-50).map((key) => [key, next[key] as string]))
}

/// git writes progress with carriage returns, and a large clone prints
/// thousands of lines: only the tail is kept and only the tail is drawn.
function progressTail(text: string) {
  return text.split(/[\r\n]+/).map((line) => line.trim()).filter(Boolean).slice(-6).join('\n')
}

function Lock() {
  return (
    <svg className="repo-lock" viewBox="0 0 24 24" width="10" height="10" aria-hidden="true">
      <path d="M12 2a5 5 0 0 0-5 5v3H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8a2 2 0 0 0-2-2h-1V7a5 5 0 0 0-5-5Zm-3 8V7a3 3 0 0 1 6 0v3H9Z" fill="currentColor" />
    </svg>
  )
}

/// Two views in one pane: what a GitHub profile holds, and what this folder is.
///
/// Browsing is the default whenever the open folder is not a GitHub checkout, so
/// the pane opens on something to do rather than on "no repository". Picking a
/// repository turns its row into the facts about it, and one click from there
/// clones it and hands the new folder back to the app as the workspace.
export function GitHubPane({ workspace, preferences, update, onOpenFolder }: Props) {
  const [mode, setMode] = useState<'browse' | 'folder'>('browse')

  // One read decides how the pane opens, and only the first one: cloning a
  // repository changes the workspace, and flipping the switch under the user at
  // that moment would hide the Open button they are about to want.
  const decided = useRef(false)
  useEffect(() => {
    if (!workspace || decided.current) return
    let alive = true
    void readRepo().then((next) => {
      if (!alive) return
      decided.current = true
      setMode(next.kind === 'repo' && /github\.com/i.test(next.repo.origin) ? 'folder' : 'browse')
    })
    return () => { alive = false }
  }, [workspace])

  const [account, setAccount] = useState<GithubAccount | null>(null)
  const [repos, setRepos] = useState<RemoteRepo[] | null>(null)
  const [listBusy, setListBusy] = useState(false)
  const [listError, setListError] = useState<string | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [login, setLogin] = useState(preferences.githubLogin)
  const [wanted, setWanted] = useState(preferences.githubLogin.trim())
  const [reloads, setReloads] = useState(0)
  const [selected, setSelected] = useState<string | null>(null)
  const [activity, setActivity] = useState<Activity | null>(null)
  const [activityBusy, setActivityBusy] = useState(false)
  const [activityError, setActivityError] = useState<string | null>(null)
  const [clone, setClone] = useState<CloneState | null>(null)

  // The list lives for the session: one read per account, Refresh or new login.
  // Signed out the limit is 60 requests an hour, so this is not polled.
  useEffect(() => {
    if (mode !== 'browse') return
    let alive = true
    setListBusy(true)
    setListError(null)
    void (async () => {
      try {
        const next = await githubAccount()
        if (!alive) return
        setAccount(next)
        // A token already answers "your repositories"; the remembered login is
        // the signed-out path, which needs no secret and works for any profile.
        const list = await githubRepos(next.signedIn ? '' : wanted)
        if (!alive) return
        setRepos(list)
        setSelected((current) => (list.some((repo) => repo.fullName === current) ? current : null))
      } catch (err) {
        if (alive) setListError(errorText(err))
      } finally {
        if (alive) setListBusy(false)
      }
    })()
    return () => { alive = false }
  }, [mode, wanted, reloads])

  const selectedRepo = repos?.find((repo) => repo.fullName === selected) ?? null

  // Three calls per repository (two counts and the last commits), so they run
  // when a row is chosen rather than for every row in the list.
  useEffect(() => {
    setActivity(null)
    setActivityError(null)
    if (!selectedRepo) return
    const slug = parseFullName(selectedRepo.fullName)
      ?? { owner: selectedRepo.owner, name: selectedRepo.name }
    let alive = true
    setActivityBusy(true)
    void githubActivity(slug.owner, slug.name)
      .then((next) => { if (alive) setActivity(next) })
      .catch((err) => { if (alive) setActivityError(errorText(err)) })
      .finally(() => { if (alive) setActivityBusy(false) })
    return () => { alive = false }
  }, [selectedRepo])

  const visible = useMemo(() => {
    const term = filter.trim().toLowerCase()
    const list = repos ?? []
    if (!term) return list
    return list.filter((repo) =>
      repo.fullName.toLowerCase().includes(term)
      || (repo.description ?? '').toLowerCase().includes(term))
  }, [repos, filter])

  const localPath = selectedRepo ? preferences.gitHubClones[selectedRepo.fullName] : undefined
  const active = selectedRepo && clone && clone.fullName === selectedRepo.fullName ? clone : null
  const cloning = Boolean(active && !active.error)

  const changeRoot = async () => {
    try {
      const picked = await cloneParentPick()
      if (picked) update({ cloneRoot: picked })
    } catch (err) {
      setFailure(errorText(err))
    }
  }

  /// Clone, then open the new folder as the workspace: the two steps the user
  /// would otherwise take by hand, and the reason this pane exists.
  const startClone = async (repo: RemoteRepo) => {
    const known = preferences.gitHubClones[repo.fullName]
    if (known) {
      onOpenFolder(known)
      return
    }
    const slug = parseFullName(repo.fullName) ?? { owner: repo.owner, name: repo.name }
    let parent = preferences.cloneRoot.trim()
    if (!parent) {
      try {
        const picked = await cloneParentPick()
        if (!picked) return
        parent = picked
      } catch (err) {
        setFailure(errorText(err))
        return
      }
    }

    setFailure(null)
    setClone({ fullName: repo.fullName, id: '', text: '', error: null })
    const job = cloneRepository({ owner: slug.owner, name: slug.name, parent }, (chunk) => {
      setClone((current) => (current && current.fullName === repo.fullName
        ? { ...current, text: (current.text + chunk).slice(-8000) }
        : current))
    })
    setClone((current) => (current ? { ...current, id: job.id } : current))

    try {
      const path = await job.done
      setClone(null)
      update({
        cloneRoot: parent,
        gitHubClones: rememberClone(preferences.gitHubClones, repo.fullName, path),
      })
      onOpenFolder(path)
    } catch (err) {
      // git already printed its reason into the progress tail above.
      const cancelled = (err as { kind?: string } | null)?.kind === 'cancelled'
      setClone((current) => (current
        ? { ...current, error: cancelled ? 'Clone cancelled.' : errorText(err) }
        : current))
    }
  }

  const openOrClone = () => {
    if (!selectedRepo) return
    if (localPath) onOpenFolder(localPath)
    else void startClone(selectedRepo)
  }

  const submitLogin = (event: FormEvent) => {
    event.preventDefault()
    const next = login.trim()
    update({ githubLogin: next })
    setWanted(next)
    setReloads((value) => value + 1)
  }

  const accountLine = account === null
    ? 'Reading account…'
    : account.signedIn
      ? '@' + (account.login ?? 'github') + (account.source ? ' · ' + account.source : '')
      : account.source
        ? 'Not signed in · the ' + account.source + ' was rejected'
        : 'Not signed in'

  const browse = (
    <div className="github-browse">
      <div className="github-account">
        <span className="github-account-line">{accountLine}</span>
        {account && !account.signedIn && (
          <form className="github-login" onSubmit={submitLogin}>
            <input value={login} spellCheck={false} placeholder="Browse public profile"
              aria-label="GitHub login"
              onChange={(event) => { setLogin(event.target.value); update({ githubLogin: event.target.value }) }} />
            <button type="submit" className="ghost">Browse</button>
          </form>
        )}
      </div>

      <div className="github-clone-root">
        <span className="github-clone-path" title={preferences.cloneRoot || undefined}>
          {preferences.cloneRoot ? 'Clones into ' + preferences.cloneRoot : 'No clone folder chosen yet'}
        </span>
        <button type="button" className="ghost" onClick={() => void changeRoot()}>Change</button>
      </div>

      <label className="repo-filter">
        <input value={filter} spellCheck={false} placeholder="Filter repositories"
          aria-label="Filter repositories"
          onChange={(event) => setFilter(event.target.value)} />
      </label>

      {listError && <p className="panel-error">{listError}</p>}
      {failure && <p className="panel-error">{failure}</p>}
      {listBusy && repos === null && <p className="hint">Reading repositories…</p>}

      {repos !== null && (
        <ul className="repo-list">
          {visible.map((repo) => (
            <li key={repo.fullName}>
              <button type="button" onClick={() => setSelected(repo.fullName)}
                className={repo.fullName === selected ? 'repo-row is-on' : 'repo-row'}>
                <span className="repo-name" title={repo.fullName}>
                  {repo.private && <Lock />}
                  {repo.name}
                </span>
                <span className="repo-owner">{repo.owner}</span>
                {repo.stars > 0 && <span className="repo-stars">{repo.stars}</span>}
                <span className="repo-when">{relativeTime(repo.updatedAt)}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {repos !== null && visible.length === 0 && (
        <p className="hint">
          {repos.length === 0 ? 'No repositories here yet.' : 'No repository matches that filter.'}
        </p>
      )}

      {selectedRepo && (
        <article className="repo-detail">
          <header className="repo-detail-head">
            <h3 className="repo-title" title={selectedRepo.fullName}>{selectedRepo.fullName}</h3>
            <button type="button" className="repo-clone" onClick={openOrClone} disabled={cloning}>
              {localPath ? 'Open' : cloning ? 'Cloning…' : 'Clone to this computer'}
            </button>
            <button type="button" className="ghost"
              onClick={() => { void openLink(selectedRepo.webUrl).catch((err) => setFailure(errorText(err))) }}>
              Open on GitHub
            </button>
          </header>
          <p className="repo-meta">
            {[
              selectedRepo.visibility.charAt(0).toUpperCase() + selectedRepo.visibility.slice(1),
              selectedRepo.language ?? 'no language',
              selectedRepo.defaultBranch,
            ].join(' · ')}
          </p>

          <nav className="repo-tabs" aria-label="Repository pages">
            <span className="repo-tab is-on" aria-current="page">Threads</span>
            <button type="button" className="repo-tab"
              onClick={() => { void openLink(selectedRepo.webUrl + '/pulls').catch((err) => setFailure(errorText(err))) }}>
              {'Pull requests' + (activity ? ' ' + activity.pulls : '')}
            </button>
            <button type="button" className="repo-tab"
              onClick={() => { void openLink(selectedRepo.webUrl + '/issues').catch((err) => setFailure(errorText(err))) }}>
              {'Issues' + (activity ? ' ' + activity.issues : '')}
            </button>
            <button type="button" className="repo-tab" title="Opens Actions on GitHub"
              onClick={() => { void openLink(selectedRepo.webUrl + '/actions').catch((err) => setFailure(errorText(err))) }}>
              Actions
            </button>
          </nav>

          <div className="repo-panel">
            {activityError && <p className="panel-error">{activityError}</p>}
            {activityBusy && activity === null && <p className="hint">Reading threads…</p>}
            {activity && activity.commits.length > 0 && (
              <ol className="commit-list">
                {activity.commits.map((commit) => (
                  <li key={commit.hash} className="commit-row">
                    <code className="commit-hash">{commit.hash.slice(0, 7)}</code>
                    <span className="commit-subject" title={commit.author}>{commit.subject}</span>
                    <span className="commit-when">{relativeTime(commit.when)}</span>
                  </li>
                ))}
              </ol>
            )}
            {activity && activity.commits.length === 0 && (
              <p className="hint">No commits on the default branch yet.</p>
            )}

            {localPath ? (
              <div className="repo-empty">
                <p className="repo-empty-title">On this computer</p>
                <p className="hint repo-path">{localPath}</p>
                <button type="button" className="repo-clone" onClick={() => onOpenFolder(localPath)}>Open</button>
              </div>
            ) : (
              <div className="repo-empty">
                <p className="repo-empty-title">Not on this computer yet</p>
                <p className="hint">
                  Clone the repository to start threads, review pull requests and work on issues.
                </p>
                <button type="button" className="repo-clone" onClick={() => void startClone(selectedRepo)}
                  disabled={cloning}>
                  {cloning ? 'Cloning…' : 'Clone repository'}
                </button>
                {selectedRepo.private && (
                  <p className="hint">
                    Private clones use the credentials git already trusts. If git refuses, run gh auth login first.
                  </p>
                )}
                {active?.error && <p className="panel-error">{active.error}</p>}
                {active && active.text && (
                  <pre className="repo-progress">{progressTail(active.text)}</pre>
                )}
                {cloning && (
                  <button type="button" className="ghost"
                    onClick={() => void killProcess(active?.id ?? '').catch(() => undefined)}>
                    Stop
                  </button>
                )}
              </div>
            )}
          </div>
        </article>
      )}
    </div>
  )

  return (
    <section className="github">
      <div className="github-modes" role="group" aria-label="GitHub view">
        <button type="button" aria-pressed={mode === 'browse'}
          className={mode === 'browse' ? 'github-mode is-on' : 'github-mode'}
          onClick={() => setMode('browse')}>Your repositories</button>
        <button type="button" aria-pressed={mode === 'folder'}
          className={mode === 'folder' ? 'github-mode is-on' : 'github-mode'}
          onClick={() => setMode('folder')}>This folder</button>
      </div>
      {mode === 'browse' ? browse : <LocalRepoView workspace={workspace} />}
    </section>
  )
}
