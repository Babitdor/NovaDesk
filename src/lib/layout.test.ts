import assert from 'node:assert/strict'
import { test } from 'node:test'
import { railMax, panelMax, panelMin, panelClearance, viewRailWidth } from './layout.ts'

test('resizable panes share the window budget and preserve the transcript clearance', () => {
  for (const width of [900, 1100, 1280, 1600]) {
    const rail = railMax(width)
    const panel = panelMax(rail, width)
    assert.ok(panel >= panelMin)
    assert.ok(width - rail - panel - viewRailWidth >= panelClearance)
    assert.ok(panel <= 860)
  }
  assert.ok(panelMax(34, 1280) > panelMax(360, 1280))
})
