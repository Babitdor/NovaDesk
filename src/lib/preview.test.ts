import assert from 'node:assert/strict'
import test from 'node:test'

import { crumbsOf, iconOf, kindOf, resolveRelative, servedByUrl } from './preview.ts'

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

test('a PDF, a video and an audio track are kinds of their own', () => {
  assert.equal(kindOf('docs/spec.pdf'), 'pdf')
  assert.equal(kindOf('clip.MP4'), 'video')
  assert.equal(kindOf('song.flac'), 'audio')
})

test('only what the loopback server has to serve asks for a URL', () => {
  assert.equal(servedByUrl('html'), true)
  assert.equal(servedByUrl('pdf'), true)
  assert.equal(servedByUrl('video'), true)
  assert.equal(servedByUrl('audio'), true)
  assert.equal(servedByUrl('markdown'), false)
  assert.equal(servedByUrl('image'), false)
  assert.equal(servedByUrl('text'), false)
})

test('the tree paints a category from the extension, and a name without one still gets a category', () => {
  assert.equal(iconOf('src/App.tsx'), 'code')
  assert.equal(iconOf('src/styles.scss'), 'style')
  assert.equal(iconOf('README.md'), 'doc')
  assert.equal(iconOf('package.json'), 'data')
  assert.equal(iconOf('assets/logo.svg'), 'image')
  assert.equal(iconOf('clip.mp4'), 'media')
  assert.equal(iconOf('bundle.zip'), 'archive')
  assert.equal(iconOf('index.html'), 'page')
  assert.equal(iconOf('Makefile'), 'file')
  assert.equal(iconOf('.gitignore'), 'file')
})

test('a breadcrumb carries the prefix each segment stands for', () => {
  assert.deepEqual(crumbsOf('docs/img/diagram.png'), [
    { name: 'docs', path: 'docs' },
    { name: 'img', path: 'docs/img' },
    { name: 'diagram.png', path: 'docs/img/diagram.png' },
  ])
  assert.deepEqual(crumbsOf('README.md'), [{ name: 'README.md', path: 'README.md' }])
  assert.deepEqual(crumbsOf(''), [])
})
