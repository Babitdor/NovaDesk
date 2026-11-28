export const APPROVAL_MODES = [
  { id: 'standard', label: 'Standard', detail: 'Review tool permissions and plans before they run.' },
  { id: 'acceptEdits', label: 'Accept edits', detail: 'Approve recognized file edits. Review commands and other tools.' },
  { id: 'plan', label: 'Plan mode', detail: 'Investigate in Codex’s read-only sandbox. No edits or Desktop actions.' },
  { id: 'fullAccess', label: 'Full access', detail: 'Approve tools and plans automatically. Questions still need answers; harness restrictions remain.' },
] as const

export type ApprovalMode = typeof APPROVAL_MODES[number]['id']

export function migrateApprovalMode(value: unknown, legacy: unknown): ApprovalMode {
  if (APPROVAL_MODES.some(mode => mode.id === value)) return value as ApprovalMode
  if (value !== undefined) return 'standard'
  return legacy === true || legacy === 'all' ? 'fullAccess' : 'standard'
}
