import assert from 'node:assert/strict'
import { test } from 'node:test'
import { fileDraft, keepFileDraft } from './fileDrafts.ts'

test('file drafts survive reopening, keep their conflict baseline, and stay scoped to a workspace', () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
  const values = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', { configurable:true, value:{
    getItem:(key:string) => values.get(key) ?? null,
    setItem:(key:string, value:string) => values.set(key, value),
    removeItem:(key:string) => values.delete(key),
  } })
  try {
    keepFileDraft('workspace A', 'src/file.ts', 'original\r\n', 'edited\r\n')
    assert.deepEqual(fileDraft('workspace A', 'src/file.ts'), { base:'original\r\n', text:'edited\r\n' })
    assert.equal(fileDraft('workspace B', 'src/file.ts'), null)
    assert.equal(fileDraft('workspace A', 'file.ts'), null)
    keepFileDraft('workspace A', 'src/file.ts', 'original\r\n', 'original\r\n')
    assert.equal(fileDraft('workspace A', 'src/file.ts'), null)
    assert.equal(values.size, 0)
    keepFileDraft('workspace A', 'src/file.ts', '', 'new')
    for (const key of values.keys()) values.set(key, '{invalid')
    assert.equal(fileDraft('workspace A', 'src/file.ts'), null)
  } finally {
    if (original) Object.defineProperty(globalThis, 'localStorage', original)
    else Reflect.deleteProperty(globalThis, 'localStorage')
  }
})
