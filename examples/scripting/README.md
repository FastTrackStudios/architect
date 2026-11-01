# example-scripting

Architect's scripting API in one small crate: a task board puts four
actions on its Action List (one read-only), and Luau scripts list, query
and run them, each run one undo step of the board's `crdt::History`. A
saved script becomes an action of its own (in ⌘K, bindable); an agent
writes, runs and saves the same scripts over MCP; a viewer's script is
refused at the line that would change the board.

The walkthrough is `docs/content/getting-started/scripting.md`.

| File | What it shows |
|---|---|
| `src/lib.rs` | the board (Loro + `History`), its actions, the authorizer, `Scripting` installed, the MCP server |
| `src/main.rs` | a CLI that runs a script against a fresh board |
| `scripts/inventory.luau` | `actions.list`, `actions.describe`, `state.query`; changes nothing |
| `scripts/close_tag.luau` | `args`, a loop and a branch; one undo step |
| `scripts/tidy.luau` | a destructive action and a conditional add; saved as "Tidy the board" |
| `scripts/broken.luau` | fails on line 9; what it changed is taken back |
| `scripts/burndown.luau` | a live script: `defer`, one step a frame |
| `tests/scripts.rs` | every script, the saved-script action, the viewer |
| `tests/mcp.rs` | the agent over MCP, and the viewer over MCP |

From the repo root:

```sh
cargo run -p example-scripting -- run  examples/scripting/scripts/close_tag.luau '{"tag":"home"}'
cargo run -p example-scripting -- run  examples/scripting/scripts/broken.luau
cargo run -p example-scripting -- save "Tidy the board" examples/scripting/scripts/tidy.luau
cargo run -p example-scripting -- live examples/scripting/scripts/burndown.luau
cargo run -p example-scripting -- mcp      # MCP on stdio, for an agent
cargo test -p example-scripting
```

Native only: the Luau runtime (mlua, built from C++ source) does not
build for wasm yet.
