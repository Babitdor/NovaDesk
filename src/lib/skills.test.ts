import assert from 'node:assert/strict'
import test from 'node:test'

import { filterSkills, skillRoots, skillsSummary, type Skill } from './skills.ts'

function skill(name: string, root: string, description = ''): Skill {
  return { name, directory: name, root, description }
}

const ROWS: Skill[] = [
  skill('adhd', '.nova/skills', 'Parallel divergent ideation for coding agents'),
  skill('old-coder', '~/.claude/skills', 'Evidence-first development'),
  skill('awwwards', '~/.claude/skills', 'Award-caliber websites'),
  skill('web-research', '~/.agents/skills', 'Structured multi-agent research'),
]

test('no root and no query keep every row, in the order Rust sent them', () => {
  assert.deepEqual(filterSkills(ROWS, { root: '', query: '' }), ROWS)
})

test('a root chip selects that root and leaves the others out', () => {
  const shown = filterSkills(ROWS, { root: '~/.claude/skills', query: '' })
  assert.deepEqual(shown.map((row) => row.name), ['old-coder', 'awwwards'])
})

test('the query matches the description as well as the name', () => {
  assert.deepEqual(filterSkills(ROWS, { root: '', query: 'research' }).map((row) => row.name),
    ['web-research'])
  assert.deepEqual(filterSkills(ROWS, { root: '', query: 'evidence' }).map((row) => row.name),
    ['old-coder'])
})

test('the query ignores case and spaces the reader leaves in the box', () => {
  assert.deepEqual(filterSkills(ROWS, { root: '', query: '  CODER ' }).map((row) => row.name),
    ['old-coder'])
  assert.deepEqual(filterSkills(ROWS, { root: '', query: '   ' }), ROWS)
})

test('the root chip and the query narrow together', () => {
  assert.deepEqual(filterSkills(ROWS, { root: '~/.claude/skills', query: 'award' }).map((row) => row.name),
    ['awwwards'])
  // The name matches, the root does not: the chip wins and the row stays out.
  assert.deepEqual(filterSkills(ROWS, { root: '.nova/skills', query: 'coder' }), [])
})

test('skillRoots lists each contributing root once, in first-seen order', () => {
  assert.deepEqual(skillRoots(ROWS), ['.nova/skills', '~/.claude/skills', '~/.agents/skills'])
  assert.deepEqual(skillRoots([]), [])
})

test('the summary says when rows are hidden and when nothing was found', () => {
  assert.equal(skillsSummary(4, 4, 3), '4 skills across 3 mounted folders.')
  assert.equal(skillsSummary(4, 1, 1), '1 of 4 skills across 1 mounted folder.')
  assert.equal(skillsSummary(0, 0, 0),
    'No skills found in the folders this workspace mounts.')
})
