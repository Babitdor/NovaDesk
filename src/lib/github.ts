import { Channel, invoke } from '@tauri-apps/api/core'

import type { AgentEvent } from './events'

/// Mirrors the DTOs in src-tauri/src/commands/github.rs. Nothing here carries a
/// credential: the token stays in Rust, and the page only ever sees metadata.
export type GithubAccount = {
  signedIn: boolean
  login: string | null
  /// Which credential answered, or was rejected: an environment variable, gh, or
  /// the token the app stored.
  source: string | null
  /// Kept so the API shape stays in one place; the page cannot draw it, because
  /// the window's CSP allows images from 'self' and data: only.
  avatarUrl: string | null
}

export type Repo = {
  owner: string
  name: string
  fullName: string
  private: boolean
  visibility: string
  defaultBranch: string
  language: string | null
  updatedAt: string
  description: string | null
  cloneUrl: string
  webUrl: string
  stars: number
}

export type CommitRow = { hash: string; subject: string; author: string; when: string }

export type Activity = { pulls: number; issues: number; commits: CommitRow[] }

export const githubAccount = () => invoke<GithubAccount>('github_account')

/// A login browses that profile's public repositories; without one the call uses
/// whichever token answered, which is what "your repositories" means.
export const githubRepos = (login?: string) =>
  invoke<Repo[]>('github_repos', { login: login || null })

export const githubActivity = (owner: string, name: string) =>
  invoke<Activity>('github_activity', { owner, name })

export const setGithubToken = (token: string | null) =>
  invoke<GithubAccount>('github_set_token', { token })

export const cloneParentPick = () => invoke<string | null>('clone_parent_pick')

export type CloneJob = { id: string; done: Promise<string> }

/// Clones owner/name into a folder and resolves with the new folder.
///
/// The command itself awaits git, so the invoke promise is the result and the
/// channel only carries progress: stderr reaches the pane while the clone runs,
/// and the same id can be handed to killProcess to stop it.
export function cloneRepository(
  request: { owner: string; name: string; parent: string | null },
  onProgress: (chunk: string) => void,
): CloneJob {
  const id = crypto.randomUUID()
  // Attached before the invoke, or a clone that starts fast loses its first
  // progress lines.
  const channel = new Channel<AgentEvent>()
  channel.onmessage = (event) => {
    if (event.type === 'processOutput') onProgress(event.chunk)
  }
  const done = invoke<string>('github_clone', { commandId: id, ...request, onEvent: channel })
  return { id, done }
}

/// "40m", "3h", "2d": the repository list's shorthand, from the API's ISO
/// timestamp. A date the API did not send renders as nothing rather than NaN.
export function relativeTime(iso: string): string {
  const then = Date.parse(iso)
  if (Number.isNaN(then)) return ''
  const minutes = Math.max(0, Math.round((Date.now() - then) / 60000))
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${minutes}m`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours}h`
  const days = Math.round(hours / 24)
  if (days < 30) return `${days}d`
  const months = Math.round(days / 30)
  if (months < 12) return `${months}mo`
  return `${Math.round(months / 12)}y`
}

export function parseFullName(full: string): { owner: string; name: string } | null {
  const [owner, name, ...rest] = full.split('/')
  if (!owner || !name || rest.length > 0) return null
  return { owner, name }
}
