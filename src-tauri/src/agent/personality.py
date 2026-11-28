"""Apply Desktop instructions inside a single Nova pipe process."""
import os
import sys

source = sys.argv.pop(1)
if source:
    sys.path.insert(0, source)
sys.argv[0] = "nova"
role = os.environ.pop("NOVA_DESKTOP_PERSONALITY", "")
instructions = os.environ.pop("NOVA_DESKTOP_SYSTEM_PROMPT", "")

# Some installed Nova versions read only the global MCPConfig in pipe mode.
# Add this session's Desktop server in memory; never modify the global profile.
credentials = os.environ.get("NOVA_DESKTOP_MCP_FILE")
shim = os.environ.get("NOVA_TOOLS_SHIM")
if credentials and shim:
    from novacode_cli.mcp.config import MCPConfig, MCPServerConfig

    load = MCPConfig.load

    def load_desktop(self):
        servers = dict(load(self))
        servers["nova-desktop"] = MCPServerConfig(
            transport="stdio", command=shim, args=[],
            env={"NOVA_TOOLS_FILE": credentials}, disabled=False,
        )
        return servers

    MCPConfig.load = load_desktop

    # An affected Nova release drops validated arguments on its success path.
    try:
        import json
        from novacode_cli.agents import code_mode
    except ImportError:
        pass
    else:
        repair_dispatch_args = getattr(getattr(code_mode, "CodeModeMiddleware", None), "_repair_dispatch_args", None)

        def desktop_arguments(self, target, arguments, schema, step):
            result = repair_dispatch_args(self, target, arguments, schema, step)
            if result is None and target.name.startswith("nova-desktop_"):
                code_mode._validator(json.dumps(schema, sort_keys=True), close_named=True).validate(arguments)
                return arguments
            return result

        if callable(repair_dispatch_args) and callable(getattr(code_mode, "_validator", None)):
            code_mode.CodeModeMiddleware._repair_dispatch_args = desktop_arguments

from novacode_cli.agents import core_agent
from novacode_cli.headless import pipe

# Pipe releases that omit the thinking event already expose it in their TUI wire.
original_frame = getattr(pipe.PipeSession, "_frame", None)


def desktop_frame(self, msg):
    if msg.get("t") == "ev" and msg.get("c") == "ReasoningDelta":
        text = msg.get("d", {}).get("text", "")
        if isinstance(text, str) and text:
            self.publish("reasoning_delta", content=text, request_id=self._turn_id)
        return
    return original_frame(self, msg)


if callable(original_frame):
    pipe.PipeSession._frame = desktop_frame


create_agent = core_agent.create_agent_with_config


def create_personality(model, assistant_id, tools, **kwargs):
    base = kwargs.get("system_prompt")
    if base is None:
        base = core_agent.get_system_prompt(
            assistant_id, kwargs.get("sandbox_type"), kwargs.get("exec_sandbox", False)
        )
    kwargs["system_prompt"] = base + "\n\n" + instructions
    return create_agent(model, assistant_id, tools, **kwargs)


async def run_personality(**kwargs):
    state = kwargs["session_state"]
    if role == "nora":
        state.browser_mode_enabled = True
    return await pipe.PipeSession(**kwargs).run()


if instructions:
    core_agent.create_agent_with_config = create_personality
    pipe.run_pipe_session = run_personality

from novacode_cli.entrypoint import cli_main

cli_main()
