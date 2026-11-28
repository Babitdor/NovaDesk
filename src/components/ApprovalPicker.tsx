import { ComposerPicker } from './ComposerPicker'
import { APPROVAL_MODES, type ApprovalMode } from '../lib/approval'

export function ApprovalIcon({ mode }: { mode: ApprovalMode }) {
  return <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
    {mode === 'standard' ? <path d="m8 1.8 5 2v4c0 3-5 6.2-5 6.2S3 10.8 3 7.8v-4z" />
      : mode === 'acceptEdits' ? <><path d="m3 10 7.7-7.7 3 3L6 13H3zM9 4l3 3" /></>
      : mode === 'plan' ? <><rect x="3.5" y="3" width="9" height="11" rx="1" /><path d="M6 3V1.5h4V3M6 7h4M6 10h4" /></>
      : <><rect x="3.5" y="7" width="9" height="6.5" rx="1" /><path d="M5.7 7V4.8a2.3 2.3 0 0 1 4.6 0" /></>}
  </svg>
}

export function ApprovalPicker({ mode, onChange, planReason }: {
  mode: ApprovalMode; onChange: (mode: ApprovalMode) => void; planReason?: string
}) {
  return <ComposerPicker name="approval" label="Approval mode" value={mode} onChange={onChange}
    options={APPROVAL_MODES.map(option => ({ ...option, icon: <ApprovalIcon mode={option.id} />,
      reason: option.id === 'plan' ? planReason : undefined }))} />
}
