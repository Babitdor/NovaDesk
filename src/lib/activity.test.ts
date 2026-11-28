import assert from 'node:assert/strict'
import { test } from 'node:test'
import { activityEntries, activityCounts, formatToolDetail, toolLabel, type ActivityItem } from './activity.ts'

test('activity compacts a tool burst, preserves order, and never crosses a reply or user message', () => {
  const tools:ActivityItem[] = [
    { id:'a', kind:'thinking', text:'Check servers', active:false },
    { id:'b', kind:'tool', callId:'b', label:'eval', input:'', ok:true },
    { id:'c', kind:'tool', callId:'c', label:'open_browser', input:'', ok:false },
    { id:'d', kind:'tool', callId:'d', label:'read_browser', input:'' },
    { id:'e', kind:'tool', callId:'e', label:'eval', input:'', finished:true },
  ]
  const entries = activityEntries([{ id:'u', kind:'user', text:'Open the site' }, ...tools,
    { id:'r', kind:'assistant', text:'Ready' }, { id:'f', kind:'tool', callId:'f', label:'eval', input:'' }])
  assert.deepEqual(entries.map((item) => item.kind), ['user', 'activity', 'assistant', 'activity'])
  assert.equal(entries[1].kind === 'activity' && entries[1].id, 'a')
  assert.deepEqual(entries[1].kind === 'activity' && entries[1].items, tools)
  assert.deepEqual(activityCounts(tools), { tools:4, failed:1, interrupted:1, running:1, thinking:false })
  assert.equal(toolLabel('Desktop · open_browser'), 'open browser')
  assert.equal(toolLabel('eval'), 'Run code')
})

test('tool labels read as words and tool payloads arrive decoded and indented', () => {
  // codex names its items in camelCase; snake_case is everyone else's convention.
  assert.equal(toolLabel('commandExecution'), 'command execution')
  assert.equal(toolLabel('mcpToolCall'), 'mcp tool call')
  assert.equal(toolLabel('LIST_LOCAL_SERVERS'), 'LIST LOCAL SERVERS')
  // A JSON string is how a diff, a file body or a script reaches the row: shown
  // raw it is one line of escaped \n in a 220px box.
  assert.equal(formatToolDetail('"@@ -1 +1 @@\\n-old\\n+new"'), '@@ -1 +1 @@\n-old\n+new')
  assert.equal(formatToolDetail('{"command":"npm run lint"}'), '{\n  "command": "npm run lint"\n}')
  assert.equal(formatToolDetail('compile output 3 warnings'), 'compile output 3 warnings')
  assert.equal(formatToolDetail('{not json'), '{not json')
})
