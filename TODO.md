# TODO

- [ ] **Dedupe search output.** Multiple hits within the same conversation region each emit their surrounding context, so large blocks of conversation appear repeatedly in one result set. Merge overlapping/adjacent hit windows per session before rendering, emit each block once (with its hit count if useful).
- [ ] **Exclude the current session from search.** Results from the session asking the question are noise — the agent already has that context. Skip the active session id (available to the hook/agent via session_id) at query time.
- [ ] **Better `rrecall` help.** `rrecall --help` needs descriptive coverage: what it indexes, query syntax, filters (session/date/project), output format, and examples — not just flag names.
