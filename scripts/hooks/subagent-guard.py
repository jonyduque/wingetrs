# PreToolUse hook (Bash|PowerShell): block destructive commands in subagents only.
# The main session (no `agent_id` in the payload) passes through; subagents get
# exit 2, which Claude Code turns into a refusal shown to the agent.
# Self-test: python -I scripts/hooks/subagent-guard.py --selftest
import json
import re
import sys

# Flags git/cargo accept before the subcommand (`git -C dir push`, `cargo +nightly update`).
GIT = r"\bgit(?:\.exe)?(?:\s+(?:-[Cc]\s+\S+|--?[\w-]+(?:=\S+)?))*\s+"
CARGO = r"\bcargo(?:\.exe)?(?:\s+(?:\+\S+|--?[\w-]+(?:=\S+)?))*\s+"

RULES = [
    (re.compile(GIT + r"push\b"), "git push"),
    (re.compile(CARGO + r"update\b"), "cargo update"),
    (re.compile(GIT + r"reset\b[^;&|\n]*--hard\b"), "git reset --hard"),
    (re.compile(GIT + r"clean\b"), "git clean"),
]


def blocked(command: str) -> str | None:
    for pattern, name in RULES:
        if pattern.search(command):
            return name
    return None


def selftest() -> int:
    hits = [
        "git push",
        "git push -u origin main",
        "cd x && git push origin v0.1.0",
        "git -C C:/repo push",
        "git -c core.x=y push",
        "cargo update",
        "cargo +nightly update -p foo",
        "git reset --hard HEAD~1",
        "git reset -q --hard",
        "git clean",
        "git clean -fdx",
        "echo a; git clean -n",
        "git.exe push",
    ]
    misses = [
        "git status",
        "git log --grep push",
        "git commit -m 'push later'",
        "git reset HEAD file.rs",
        "git reset --soft HEAD~1",
        "cargo test --quiet update_flow",
        "cargo build",
        "rg cleanup src",
        "git reset file; echo --hard",
    ]
    bad = [c for c in hits if blocked(c) is None] + [c for c in misses if blocked(c)]
    for c in bad:
        print(f"[FALHA] {c}")
    print("[OK] selftest" if not bad else f"{len(bad)} falha(s)")
    return 1 if bad else 0


def main() -> int:
    if sys.argv[1:] == ["--selftest"]:
        return selftest()
    try:
        payload = json.load(sys.stdin)
    except ValueError:
        return 0
    if not payload.get("agent_id"):
        return 0
    command = str((payload.get("tool_input") or {}).get("command", ""))
    name = blocked(command)
    if name is None:
        return 0
    sys.stderr.reconfigure(encoding="utf-8")
    print(
        f"Bloqueado para subagentes: `{name}` (CLAUDE.md, comandos proibidos). "
        "Peça ao orquestrador.",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    sys.exit(main())
