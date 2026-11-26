/// Unsent composer text, kept only while "Keep unsent drafts" is on.
///
/// The store is one localStorage object so clearing a draft is a single write
/// and a stale entry cannot resurrect itself in another tab. Entries expire
/// after 30 days and the whole store is capped, because a draft is a kindness,
/// not a database: a prompt parked a month ago is no longer what anyone means.

const KEY = 'nova.drafts.v1'
const MAX_AGE = 30 * 24 * 60 * 60 * 1000
const MAX_CHARS = 200_000

type Draft = { text: string; at: number }

export function loadDrafts(now = Date.now()): Record<string, string> {
  const stored = read()
  const kept: Record<string, Draft> = {}
  for (const [chatId, draft] of Object.entries(stored)) {
    if (draft && typeof draft.text === 'string' && draft.text.trim()
        && now - draft.at <= MAX_AGE && now - draft.at >= 0) {
      kept[chatId] = draft
    }
  }
  if (Object.keys(kept).length !== Object.keys(stored).length) write(kept)
  return Object.fromEntries(Object.entries(kept).map(([chatId, draft]) => [chatId, draft.text]))
}

export function saveDraft(chatId: string, text: string, now = Date.now()) {
  const stored = read()
  if (!text.trim()) delete stored[chatId]
  else stored[chatId] = { text: text.slice(0, MAX_CHARS), at: now }
  write(stored)
}

export function clearDraft(chatId: string) {
  const stored = read()
  if (chatId in stored) {
    delete stored[chatId]
    write(stored)
  }
}

export function clearAllDrafts() {
  try { localStorage.removeItem(KEY) } catch { /* nothing stored is the same outcome */ }
}

/// How many drafts are waiting, for the "Clear saved drafts" row.
export function draftCount(now = Date.now()): number {
  return Object.keys(loadDrafts(now)).length
}

function read(): Record<string, Draft> {
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
    return raw as Record<string, Draft>
  } catch {
    // A corrupt store is not worth failing a keystroke over: start clean.
    return {}
  }
}

function write(stored: Record<string, Draft>) {
  // Newest drafts win when the cap is reached: the oldest are dropped first.
  const entries = Object.entries(stored).sort(([, a], [, b]) => b.at - a.at)
  let budget = MAX_CHARS
  const kept: Record<string, Draft> = {}
  for (const [chatId, draft] of entries) {
    if (!draft || typeof draft.text !== 'string') continue
    if (draft.text.length > budget) continue
    budget -= draft.text.length
    kept[chatId] = draft
  }
  try {
    if (Object.keys(kept).length) localStorage.setItem(KEY, JSON.stringify(kept))
    else localStorage.removeItem(KEY)
  } catch { /* storage full or disabled: drafts are best effort */ }
}
