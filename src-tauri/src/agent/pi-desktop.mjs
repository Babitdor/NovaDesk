export default function (pi) {
  if (process.env.NOVA_TOOLS_FILE && typeof pi.registerMcpServer === 'function') {
    pi.registerMcpServer('nova-desktop', {
      type: 'stdio', command: process.env.NOVA_TOOLS_SHIM,
      env: { NOVA_TOOLS_FILE: process.env.NOVA_TOOLS_FILE }, exposure: 'direct',
    })
  }
  pi.on('tool_call', async (event, ctx) => {
    if (ctx.mode !== 'rpc' || !ctx.hasUI) return { block:true, reason:'Interactive Desktop permissions are unavailable in this CLI mode' }
    if (!await ctx.ui.confirm('Desktop tool permission', JSON.stringify({ tool: event.toolName, input: event.input }))) {
      return { block: true, reason: 'Denied in Nova Desktop' }
    }
  })
}
