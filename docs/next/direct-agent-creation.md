# Direct agent creation

The custom fork can create an agent directly in a new worktree root, tab, or split. It uses the existing argv process launcher and managed agent startup lifecycle; it does not submit a command to an interactive shell.

The narrow JSON methods are `worktree.create_agent`, `worktree.open_agent`, `tab.create_agent`, and `pane.split_agent`. Their creation parameters match the corresponding existing method, with an `agent` object containing `name`, `kind`, `command` (an argv array), `env`, and optional `tab_label` and `timeout_ms`. The CLI selects these methods only when `--agent-launch JSON` is supplied. Older methods and clients keep their shell creation behavior. Servers without the new methods reject the request before creating anything; the client never falls back to shell input.

The executable must be the requested agent executable name or an absolute path to that executable. Arguments and environment values are separate strings, never a shell script. For a tab or split, the structured agent environment overrides the corresponding creation environment key. Startup registration uses the existing managed name, detection, session, deadline, and interactive readiness checks. Creation acknowledges the exact new terminal, pane, workspace, and tab. A failed or unknown outcome must be inspected, not automatically retried.

Opening an already open worktree with direct launch rejects the request and preserves its root. A race that opens the checkout while creation is pending also rejects direct launch. New tabs and splits preserve existing terminals. Split launch verifies the requested workspace and tab label before creating the new pane. No existing tab replacement, shell reuse, automatic close, or background monitoring is introduced.
