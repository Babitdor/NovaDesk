import { CREW, crewRoles, systemPrompt } from '../lib/crew'
import { AGENTS, type AgentId, type Preferences } from '../lib/settings'
import { CrewPortrait } from './CrewPortrait'
import { Group, Toggle } from './SettingsRows'
const agentIds = Object.keys(AGENTS) as AgentId[]
export function TeamMembers({ preferences:p, update }: { preferences:Preferences; update:(patch:Partial<Preferences>) => void }) {
  return <>
              <Group title="Team pipeline">
                <p className="settings-help">Nova coordinates bounded assignments and reports the results. Enabled workers form the fallback sequence. Only enforced read-only investigations run concurrently; editing turns run one at a time.</p>
                <Toggle title="Require plan approval" detail="Pause the run after Nova's plan until you approve, edit or skip it."
                  checked={p.team.planApproval}
                  onChange={(enabled) => update({ team: { ...p.team, planApproval: enabled } })} />
                <label className="settings-row">
                  <span><strong>Ralph iterations</strong><small>How often a genuinely failing check is retried before the run moves on.</small></span>
                  <select aria-label="Ralph iterations" value={p.team.ralphIterations}
                    onChange={(event) => update({ team: { ...p.team, ralphIterations: Number(event.target.value) } })}>
                    {[1, 2, 3, 4, 5].map((count) => <option key={count} value={count}>{count}</option>)}
                  </select>
                </label>
                <label className="settings-row">
                  <span><strong>Stage watchdog</strong><small>Fail a stage that produces no output for this long, and let the team continue.</small></span>
                  <select aria-label="Stage watchdog" value={p.team.stageTimeoutMin}
                    onChange={(event) => update({ team: { ...p.team, stageTimeoutMin: Number(event.target.value) } })}>
                    {[2, 5, 10, 15].map((minutes) => <option key={minutes} value={minutes}>{minutes} min</option>)}
                  </select>
                </label>
                <div className="settings-row">
                  <span><strong>Pipeline workers</strong><small>Which agents run between Nova's plan and the final report.</small></span>
                  <span className="crew-pipeline-roles">
                    {crewRoles.filter((role) => role !== 'nova').map((role) => {
                      const enabled = p.team.roles[role] !== false
                      return <label key={role} className="crew-role-toggle">
                        <input type="checkbox" checked={enabled} onChange={(event) => update({
                          team: { ...p.team, roles: { ...p.team.roles, [role]: event.target.checked } },
                        })} />
                        <CrewPortrait role={role} />
                        {CREW[role].name}
                      </label>
                    })}
                  </span>
                </div>
              </Group>
              <Group title="Agent personalities">
              <p className="settings-help">Address a message with @nova, @serena, @ralph, @nora, or @team. Each personality uses the harness, model, and system prompt set here.</p>
              {crewRoles.map((role) => {
                const profile = p.crew[role] ?? { agent: 'nova' }
                const actor = CREW[role]
                return <section className="crew-profile" key={role}>
                  <header className="reply-speaker">
                    <CrewPortrait role={role} />
                    <div><strong>{actor.name}</strong><small>{actor.title}</small></div>
                  </header>
                  {role === 'nora' && <p>Browser research uses the browser and search tools configured in the selected harness.</p>}
                  <div className="crew-profile-fields">
                    <label>Harness<select aria-label={actor.name + ' harness'} value={profile.agent}
                      onChange={(event) => update({ crew: { ...p.crew, [role]: {
                        ...profile, agent: event.target.value as AgentId, model: undefined,
                      } } })}>
                      {agentIds.map((id) => <option key={id} value={id}>{AGENTS[id].name}</option>)}
                    </select></label>
                    <label>Model<input aria-label={actor.name + ' model'} value={profile.model ?? ''}
                      placeholder={AGENTS[profile.agent].models + ' or harness default'} maxLength={200}
                      onChange={(event) => update({ crew: { ...p.crew, [role]: { ...profile, model: event.target.value } } })} /></label>
                  </div>
                  <label>System prompt<textarea aria-label={actor.name + ' system prompt'} rows={3}
                    value={profile.systemPrompt ?? systemPrompt(role)} maxLength={2000}
                    placeholder="Use the default personality prompt"
                    onChange={(event) => update({ crew: { ...p.crew, [role]: { ...profile, systemPrompt: event.target.value } } })} /></label>
                </section>
              })}
              </Group>
            </>
}
