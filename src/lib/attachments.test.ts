import { test } from 'node:test'
import assert from 'node:assert/strict'
import { attachmentPrompt } from './attachments.ts'

test('an empty draft becomes just the paths', () => {
  assert.equal(attachmentPrompt('', ['src/a.ts', 'src/b.ts']), 'src/a.ts src/b.ts')
})

test('paths follow existing text with exactly one separating space', () => {
  assert.equal(attachmentPrompt('fix this  \n', ['src/a.ts']), 'fix this src/a.ts')
  assert.equal(attachmentPrompt('   ', ['src/a.ts']), 'src/a.ts')
})

test('a path with whitespace is quoted, an already quoted one is left alone', () => {
  assert.equal(attachmentPrompt('', ['C:/My Files/a.ts']), '"C:/My Files/a.ts"')
  assert.equal(attachmentPrompt('', ['"C:/My Files/a.ts"']), '"C:/My Files/a.ts"')
})

test('no paths is a no-op, even with trailing whitespace', () => {
  assert.equal(attachmentPrompt('half typed  ', []), 'half typed  ')
})

test('folder and drawing paths keep the same editable prompt format', () => {
  assert.equal(attachmentPrompt('Inspect', ['./', 'design notes/', '.nova/attachments/sketch-1.png']), 'Inspect ./ "design notes/" .nova/attachments/sketch-1.png')
})
