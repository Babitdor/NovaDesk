import assert from 'node:assert/strict'
import test from 'node:test'

import { kindOf, resolveRelative } from './preview.ts'

test('the extension alone decides the kind', () => {
  assert.equal(kindOf('assets/logo.svg'), 'image')
  assert.equal(kindOf('shot.PNG'), 'image')
  assert.equal(kindOf('README.md'), 'markdown')
  assert.equal(kindOf('docs/guide.markdown'), 'markdown')
  assert.equal(kindOf('site/index.html'), 'html')
  assert.equal(kindOf('src/App.tsx'), 'text')
  assert.equal(kindOf('Makefile'), 'text')
  assert.equal(kindOf('.gitignore'), 'text')
})

test('a sibling link resolves against the file it was written in', () => {
  assert.equal(resolveRelative('docs/README.md', 'img/diagram.png'), 'docs/img/diagram.png')
  assert.equal(resolveRelative('README.md', 'img/logo.png'), 'img/logo.png')
})

test('a parent segment climbs and then normalizes', () => {
  assert.equal(resolveRelative('docs/a/page.md', '../img/x.png'), 'docs/img/x.png')
  assert.equal(resolveRelative('docs/a/page.md', './b.md'), 'docs/a/b.md')
})

test('backslashes are separators, as they are on disk', () => {
  assert.equal(resolveRelative('docs/page.md', '..\\img\\x.png'), 'img/x.png')
})

test('a link that climbs out of the workspace resolves to nothing', () => {
  assert.equal(resolveRelative('README.md', '../secret.txt'), null)
  assert.equal(resolveRelative('docs/page.md', '../../outside.txt'), null)
})

test('an absolute URL, a data URI, an anchor and a mail link are not files', () => {
  assert.equal(resolveRelative('README.md', 'https://example.com/x.png'), null)
  assert.equal(resolveRelative('README.md', '//example.com/x.png'), null)
  assert.equal(resolveRelative('README.md', 'data:image/png;base64,AAAA'), null)
  assert.equal(resolveRelative('README.md', 'mailto:someone@example.com'), null)
  assert.equal(resolveRelative('README.md', '#install'), null)
  assert.equal(resolveRelative('README.md', ''), null)
})

test('a query or fragment is stripped before the path is resolved', () => {
  assert.equal(resolveRelative('docs/page.md', 'img/x.png?v=2'), 'docs/img/x.png')
  assert.equal(resolveRelative('docs/page.md', 'other.md#usage'), 'docs/other.md')
})

test('a percent-encoded name is decoded, and an absolute path means the root', () => {
  assert.equal(resolveRelative('docs/page.md', 'my%20file.md'), 'docs/my file.md')
  assert.equal(resolveRelative('docs/page.md', '/assets/x.png'), 'assets/x.png')
})
