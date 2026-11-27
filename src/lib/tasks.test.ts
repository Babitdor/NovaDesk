import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createChat, type Chat } from './sessions.ts'
import { PAGE, taskTools, type TaskDeps } from './tasks.ts'

test('task tools read, search, update, open and start the named task', async () => {
  const chat: Chat = { ...createChat(), title: 'Fixture' }
  chat.items = Array.from({ length: PAGE + 1 }, (_, index) => ({
    id: String(index), kind: 'user', text: `message ${index}`,
  }))
  const chats = [chat]
  const calls: Array<[string, string]> = []
  let activeId = chat.id
  let live = false
  const deps: TaskDeps = {
    chats: () => chats, activeId: () => activeId, liveChat: () => live ? chat.id : null,
    open: (id) => { activeId = id },
    create: (title, agent) => {
      const created = { ...createChat(), title, agent }
      chats.push(created)
      return created
    },
    update: (id, patch) => Object.assign(chats.find((task) => task.id === id)!, patch),
    archive: (id) => {
      if (live && id === chat.id) return 'Stop the running turn'
      chats.splice(chats.findIndex((task) => task.id === id), 1)
      return null
    },
    start: async (id, text) => {
      if (live) return false
      calls.push([id, text])
      return true
    },
  }
  const tool = taskTools(deps)
  assert.equal((await tool('list_tasks', {}) as { tasks: unknown[] }).tasks.length, 1)
  assert.equal((await tool('read_task', { taskId: chat.id }) as { nextOffset: number }).nextOffset, PAGE)
  assert.equal((await tool('read_task', { taskId: chat.id, offset: PAGE }) as { messages: unknown[] }).messages.length, 1)
  assert.equal((await tool('search_tasks', { query: 'MESSAGE 40' }) as { hits: unknown[] }).hits.length, 1)
  const { taskId } = await tool('create_task', { title: 'Second', agent: 'constructor' }) as { taskId: string }
  assert.equal(chats[1].agent, undefined)
  await tool('rename_task', { taskId, title: 'Renamed' })
  await tool('pin_task', { taskId, pinned: true })
  await tool('set_task_stage', { taskId, column: 'done' })
  assert.deepEqual([chats[1].title, chats[1].pinned, chats[1].column], ['Renamed', true, 'done'])
  await assert.rejects(tool('open_task', { taskId: 'missing' }), /no task/)
  assert.deepEqual(await tool('open_task', { taskId }), { opened: taskId })
  await tool('start_task', { prompt: 'Start' })
  await tool('send_message', { taskId: chat.id, text: 'Continue' })
  assert.deepEqual(calls, [[taskId, 'Start'], [chat.id, 'Continue']])
  live = true
  await assert.rejects(tool('send_message', { taskId, text: 'Busy' }), /could not be started/)
  await assert.rejects(tool('archive_task', { taskId: chat.id }), /running turn/)
  await tool('archive_task', { taskId })
  assert.deepEqual(chats, [chat])
})
