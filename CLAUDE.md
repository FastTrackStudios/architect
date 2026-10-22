# architect: agent instructions

- The `architect` agent skill lives in `skills/architect/` (`SKILL.md` + `references/*.md`); `.claude/skills/architect` links to it. Downstream fleet agents learn the framework from it.
- A PR that adds, renames or removes public API or a feature flag updates `skills/architect/` in the same PR.
- Cite code in docs and skills as a repo path in a code span plus the symbol name, never a line number; write other repos' paths as `task:…` / `studio:…`. `cargo xtask docs` (CI) fails on any path, recipe, xtask command or package that does not exist.
