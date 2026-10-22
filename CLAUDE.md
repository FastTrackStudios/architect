# architect: agent instructions

- The `architect` agent skill lives in `skills/architect/` (`SKILL.md` + `references/*.md`); `.claude/skills/architect` links to it. Downstream fleet agents learn the framework from it.
- A PR that adds, renames or removes public API or a feature flag updates `skills/architect/` in the same PR.
- Cite code in docs and skills as a repo path in a code span, followed by the symbol name when the citation is about one item (a whole module or test file can stand alone); never a line number. Write other repos' paths as `task:…` / `studio:…`. `cargo xtask docs` (CI) fails on any path, recipe, xtask command or package that does not exist.
