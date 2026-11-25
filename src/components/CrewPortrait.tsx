import type { CrewRole } from '../lib/crew'

export function CrewPortrait({ role }: { role: CrewRole }) {
  return <span className="crew-portrait" data-portrait={role} aria-hidden="true" />
}
