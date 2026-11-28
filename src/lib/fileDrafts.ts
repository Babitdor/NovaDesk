export type FileDraft = { base: string; text: string }
const key = (root: string, path: string) => 'nova.file-draft.v1:' + JSON.stringify([root, path])

export function fileDraft(root: string, path: string): FileDraft | null {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key(root, path)) ?? 'null')
    if (value && typeof value === 'object' && 'base' in value && 'text' in value
        && typeof value.base === 'string' && typeof value.text === 'string') return { base:value.base, text:value.text }
  } catch { /* An unreadable draft does not prevent opening the disk file. */ }
  return null
}

export function keepFileDraft(root: string, path: string, base: string, text: string) {
  if (base === text) localStorage.removeItem(key(root, path))
  else localStorage.setItem(key(root, path), JSON.stringify({ base, text }))
}
