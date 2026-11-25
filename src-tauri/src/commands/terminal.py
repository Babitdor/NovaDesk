"""Windows console bridge. Stdin carries control frames, stdout carries screen updates."""

import json
import os
from pathlib import Path
import sys
import threading
import time

try:
    import pyte
    from winpty import PtyProcess
except ImportError as error:
    sys.exit(f"The terminal dock requires pyte and pywinpty in Nova's Python environment ({error}).")


def git_bash():
    # Windows may expose a WSL bash.exe on PATH. Find the one shipped beside Git.
    for directory in os.get_exec_path():
        git = Path(directory.strip('"')) / "git.exe"
        if git.is_absolute() and git.is_file():
            for root in (git.parent, git.parent.parent, git.parent.parent.parent):
                bash = root / "bin" / "bash.exe"
                if (root / "git-bash.exe").is_file() and bash.is_file():
                    return bash
    for key, suffix in (
        ("ProgramFiles", "Git"),
        ("ProgramFiles(x86)", "Git"),
        ("LOCALAPPDATA", "Programs/Git"),
    ):
        if base := os.environ.get(key):
            bash = Path(base) / suffix / "bin/bash.exe"
            if bash.is_absolute() and bash.is_file():
                return bash
    sys.exit("Git Bash was not found. Install Git for Windows or add its Git executable to PATH, then open a new tab.")


rows, cols = map(int, sys.argv[1:3])
kind = sys.argv[3] if len(sys.argv) > 3 else "powershell"
system = Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32"
environment = None
if kind == "gitbash":
    command = [str(git_bash()), "--login", "-i"]
    environment = {**os.environ, "CHERE_INVOKING": "1", "TERM": "xterm"}
elif kind == "cmd":
    command = [str(system / "cmd.exe"), "/D"]
elif kind == "powershell":
    shell = system / "WindowsPowerShell/v1.0/powershell.exe"
    command = [str(shell), "-NoLogo", "-NoProfile", "-NoExit", "-Command", "Remove-Module PSReadLine -ErrorAction SilentlyContinue"]
else:
    sys.exit("Unsupported terminal shell.")

cwd = os.getcwd()
if cwd.startswith("\\\\?\\UNC\\"):
    cwd = "\\\\" + cwd[8:]
elif cwd.startswith("\\\\?\\") and cwd[5:7] == ":\\":
    cwd = cwd[4:]
terminal = PtyProcess.spawn(
    command,
    cwd=cwd,
    env=environment,
    dimensions=(rows, cols),
)
screen = pyte.HistoryScreen(cols, rows, history=300)
stream = pyte.Stream(screen)
lock = threading.Lock()
dirty = threading.Event()
finished = threading.Event()


def emit(frame):
    print(json.dumps(frame, ensure_ascii=True), flush=True)


def paint():
    while True:
        dirty.wait()
        dirty.clear()
        time.sleep(0.035)
        with lock:
            history = [
                "".join(line[col].data if col in line else " " for col in range(screen.columns)).rstrip()
                for line in screen.history.top
            ]
            lines = history + [line.rstrip() for line in screen.display]
            frame = {
                "type": "screen",
                "lines": lines,
                "row": len(history) + screen.cursor.y,
                "column": screen.cursor.x,
            }
            exited = finished.is_set()
        emit(frame)
        if exited:
            emit({"type": "exit"})
            return


def receive():
    try:
        while True:
            data = terminal.read(8192)
            if data:
                with lock:
                    stream.feed(data)
                dirty.set()
    except (EOFError, OSError):
        with lock:
            finished.set()
        dirty.set()


threading.Thread(target=paint, daemon=True).start()
threading.Thread(target=receive, daemon=True).start()
try:
    for line in sys.stdin:
        frame = json.loads(line)
        if frame["type"] == "close":
            break
        if frame["type"] == "input":
            terminal.write(frame["text"])
        if frame["type"] == "resize":
            rows, cols = frame["rows"], frame["cols"]
            with lock:
                screen.resize(lines=rows, columns=cols)
                terminal.setwinsize(rows, cols)
            dirty.set()
finally:
    terminal.close(force=True)
