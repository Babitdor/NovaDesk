# Harness marks

One file per harness, fetched from that vendor's own brand page or public
repository on 2026-11-25. They are kept byte-for-byte as published (only the
`nova.svg` spark is ours), so nothing here is redrawn or recoloured.

| File | Harness | Source |
| --- | --- | --- |
| `nova.svg` | Nova (this app) | drawn here |
| `pi.svg` | Pi Agent (`pi`, pi-mono) | https://raw.githubusercontent.com/earendil-works/pi/main/apps/p2/public/icons/favicon.svg |
| `claude.svg` | Claude Code | https://claude.ai/favicon.svg |
| `openai.svg` | Codex (OpenAI) | https://openai.com/favicon.svg |
| `opencode.svg` | OpenCode | https://opencode.ai/brand/files/opencode-logo-transparent-dark.svg |
| `commandcode.svg` | CommandCode | https://raw.githubusercontent.com/CommandCodeAI/command-code/refs/heads/main/.github/commandcode/symbols/symbol.svg |
| `hermes.png` | Hermes (Nous Research) | https://raw.githubusercontent.com/NousResearch/hermes-agent/main/apps/bootstrap-installer/src-tauri/icons/128x128.png |

Each mark is a trademark of its owner and is used here only to identify the tool
it belongs to — Nova Desktop is not affiliated with, endorsed by or sponsored by
these projects. `pi.svg` is the pixel-art favicon published by pi-mono; `openai.svg`
is OpenAI's favicon (it ships its own light/dark swap); `opencode.svg` is the
"transparent dark" variant from the OpenCode brand page, which is the one meant for
a dark background. If a vendor asks for their mark to be removed, delete the file
and add a `GLYPHS` entry in `src/components/AgentMark.tsx` instead.
