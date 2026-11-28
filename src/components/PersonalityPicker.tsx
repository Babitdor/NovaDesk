import { CREW, crewRoles, type CrewMode } from '../lib/crew'
import { ComposerPicker } from './ComposerPicker'
import { CrewPortrait } from './CrewPortrait'
import { SettingsIcon } from './SettingsRows'

export function PersonalityPicker({ crew, onChange, disabled }: {
  crew?: CrewMode; onChange: (crew?: CrewMode) => void; disabled: boolean
}) {
  return <ComposerPicker<'' | CrewMode> name="personality" label="Agent personality" value={crew ?? ''}
    onChange={value => onChange(value || undefined)} disabled={disabled} options={[
      { id: '', label: 'No personality', detail: 'Use the selected harness and model.', icon: <SettingsIcon name="agents" /> },
      ...crewRoles.map(role => ({ id: role, label: CREW[role].name, detail: CREW[role].title, icon: <CrewPortrait role={role} /> })),
      { id: 'team', label: 'Team', detail: 'Nova coordinates Serena, Ralph and Nora.',
        icon: <span className="crew-team-avatar" aria-hidden="true">{crewRoles.map(role => <CrewPortrait key={role} role={role} />)}</span> },
    ]} />
}
