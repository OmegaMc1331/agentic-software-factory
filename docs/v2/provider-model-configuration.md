# Provider and model configuration

A provider account, CLI executable, adapter contract, model identifier, profile and execution session are distinct. P09 references existing server-configured installations and reuses their authentication. It neither stores API keys in profile rows nor introduces a model gateway, remote catalog or account-switching mechanism.

Existing adapters are Codex, Claude Code, OpenCode, Gemini CLI, Qwen Code and server-defined Custom commands. Known model-selecting adapters receive a structured `--model` argument; Custom installations support their configured default only. Codex additionally accepts the bounded reasoning-effort setting through its documented configuration argument. Trusted sandbox and approval arguments remain in force. Unsupported or unsafe installation argument layouts are refused for profile execution. Profile data cannot add arbitrary CLI switches.

Model IDs permit a bounded ASCII identifier alphabet; whitespace, leading switches, control characters and command syntax are rejected. Syntactic validity means configured, not provider-verified. No universal model catalog is maintained. Executable availability is a local path-resolution observation and does not prove authentication, model access or quota.

The selected model reaches the actual invocation. Codex native headers, when present, provide a separately retained model observation. A reported model differing from the selected identifier rejects the result. Other adapters can remain unverified when they do not report trustworthy execution metadata. Native usage-limit classification and P07-R1 permission classifications remain authoritative. No provider/model substitution occurs after a failed invocation. Automatic routing across an explicitly eligible role pool is distinct from retrying a failed request through a different account or provider.

Role preference and automatic-delegation eligibility define the default routing pool. Explicit teams and task pins remain authoritative. Eligible alternatives may be selected before execution when capacity is occupied. Provider refusal does not authorize silent model fallback or repeated quota probes.

Adapter contracts are capability declarations, not cross-provider qualification. The delivery report records exactly which installations, versions, modes and native platforms executed. A third-party CLI being selectable does not imply that its current installation or every advertised model has been qualified.

Codex reference: [CLI commands and model flag](https://learn.chatgpt.com/docs/developer-commands?surface=cli), [configuration precedence and reasoning effort](https://learn.chatgpt.com/docs/config-file/config-reference). Local CLI help is captured during acceptance. Additional adapters retain their existing authentication, prompt transport and process-supervision contracts.

Adapter model-flag references: [Claude Code](https://code.claude.com/docs/en/cli-reference), [OpenCode](https://opencode.ai/docs/cli/), [Gemini CLI](https://geminicli.com/docs/cli/cli-reference/), [Qwen Code](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/). These contracts do not imply that the corresponding tools were installed or executed locally.
