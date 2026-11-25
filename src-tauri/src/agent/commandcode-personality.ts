export default function (cmd) {
  const systemPrompt = process.env.NOVA_DESKTOP_SYSTEM_PROMPT
  delete process.env.NOVA_DESKTOP_SYSTEM_PROMPT
  cmd.hooks({
    appendSystemPrompt: () => systemPrompt,
  })
}
