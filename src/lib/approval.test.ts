import assert from 'node:assert/strict'
import { test } from 'node:test'
import { migrateApprovalMode } from './approval.ts'

test('approval modes preserve explicit choices and migrate old settings without widening unknown values', () => {
  for (const mode of ['standard', 'acceptEdits', 'plan', 'fullAccess']) assert.equal(migrateApprovalMode(mode, false), mode)
  assert.equal(migrateApprovalMode(undefined, true), 'fullAccess')
  assert.equal(migrateApprovalMode(undefined, 'all'), 'fullAccess')
  for (const legacy of [false, 'tools', 'ALL', 'unknown', undefined]) assert.equal(migrateApprovalMode(undefined, legacy), 'standard')
  for (const invalid of ['unknown', true, null, {}, []]) assert.equal(migrateApprovalMode(invalid, true), 'standard')
})
