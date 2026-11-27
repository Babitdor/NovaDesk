/// The Skills tab's list: what `tools_skills` reported, narrowed to what the
/// search box and the root chips ask for, and nothing else.
///
/// `tools_skills` already resolved duplicates — a project copy of a skill wins
/// over a home copy of the same name — so nothing here merges or dedupes; it
/// only selects. The order is the order Rust sent (sorted by name), and this
/// keeps it, because re-sorting on every keystroke makes rows jump while the
/// reader is typing.
///
/// No Tauri import lives in this module: the pane calls `invoke` itself, and
/// keeping the import out is what lets `node --test` exercise the filtering.

/// One skill directory, as `tools_skills` reports it. `root` is the label of
/// the folder it came from (`.nova/skills`, `~/.claude/skills`, …), which is
/// what the scope chips filter on.
export type Skill = {
  name: string
  directory: string
  root: string
  description: string
  /// False when the user switched it off. It stays in the list so the switch
  /// can be found again, and Rust leaves it out of what a harness is handed.
  enabled: boolean
}

export type Skills = { skills: Skill[]; count: number }

/// Every root that contributed a skill, in the order the list reports them:
/// the project first, then the home folders. A root with nothing in it cannot
/// become a chip, so no chip filters the list down to nothing.
export function skillRoots(skills: Skill[]): string[] {
  const roots: string[] = []
  for (const skill of skills) {
    if (!roots.includes(skill.root)) roots.push(skill.root)
  }
  return roots
}

export type SkillFilter = {
  /// A root label, or the empty string for every root.
  root: string
  /// Free text matched against the name and the description. A blank or
  /// whitespace-only query keeps every row.
  query: string
}

/// The rows the list shows, in the order they arrived.
export function filterSkills(skills: Skill[], filter: SkillFilter): Skill[] {
  const needle = filter.query.trim().toLowerCase()
  return skills.filter((skill) => {
    if (filter.root && skill.root !== filter.root) return false
    if (!needle) return true
    return skill.name.toLowerCase().includes(needle)
      || skill.description.toLowerCase().includes(needle)
  })
}

/// The disabled list after flipping one skill. The tab and the sheet both write
/// through this, so the two switches cannot disagree about what to store.
export function withSkillEnabled(disabled: string[], directory: string, enabled: boolean): string[] {
  if (enabled) return disabled.filter((id) => id !== directory)
  return disabled.includes(directory) ? disabled : [...disabled, directory]
}

/// The line above the rows: how many are shown out of how many exist, and
/// across how many roots. Written here rather than inline so the plural and
/// the empty case are checked instead of eyeballed.
export function skillsSummary(total: number, shown: number, roots: number): string {
  const scope = roots === 1 ? '1 mounted folder' : `${roots} mounted folders`
  if (!total) return 'No skills found in the folders this workspace mounts.'
  if (shown === total) return `${total} skills across ${scope}.`
  return `${shown} of ${total} skills across ${scope}.`
}
