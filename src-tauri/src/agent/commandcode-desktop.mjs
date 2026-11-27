import { readFile, unlink } from 'node:fs/promises'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'
import { setTimeout } from 'node:timers/promises'

export default function (cmd) {
  const systemPrompt = process.env.NOVA_DESKTOP_SYSTEM_PROMPT
  cmd.hooks({
    appendSystemPrompt: () => systemPrompt,
    onPermissionRequest: async (request) => {
      // Safety prompts stay with the harness; a mod cannot override them.
      if (request.isSafetyPrompt) return 'deny'
      const id = randomUUID()
      const path = join(process.env.NOVA_DESKTOP_DECISIONS, id + '.json')
      process.stdout.write(JSON.stringify({ type: 'desktop_permission', id, request }) + '\n')
      for (;;) {
        try {
          const reply = JSON.parse(await readFile(path, 'utf8'))
          await unlink(path)
          return reply.approve === true ? 'allow' : 'deny'
        } catch (error) {
          if (error.code !== 'ENOENT') throw error
        }
        await setTimeout(100)
      }
    },
  })
}
