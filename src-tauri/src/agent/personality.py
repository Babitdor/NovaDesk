"""Apply Desktop instructions inside a single Nova pipe process."""
import os
import sys

source = sys.argv.pop(1)
if source:
    sys.path.insert(0, source)
sys.argv[0] = "nova"
role = os.environ.pop("NOVA_DESKTOP_PERSONALITY")
instructions = os.environ.pop("NOVA_DESKTOP_SYSTEM_PROMPT")

from novacode_cli.agents import core_agent
from novacode_cli.headless import pipe

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


core_agent.create_agent_with_config = create_personality
pipe.run_pipe_session = run_personality

from novacode_cli.entrypoint import cli_main

cli_main()
