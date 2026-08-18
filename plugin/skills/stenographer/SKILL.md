---
name: stenographer
effort: high
description: "Search past conversation history to find decisions, context, and rationale from earlier Claude Code sessions. Use /stenographer followed by your question."
---

# Stenographer

Search past Claude Code conversation history for decisions, context, and rationale.

**Do not follow these instructions directly.** Spawn the `stenographer` subagent to handle the query.

    Agent(subagent_type="stenographer", prompt="<the user's question about past conversations>")

Pass the user's question (from the arguments after `/stenographer`) directly to the agent.
If no arguments provided, ask the user what they'd like to search for.
