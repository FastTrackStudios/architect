+++
title = "Spec coverage"
description = "Per-feature spec rules tracked by tracey."
weight = 51
+++

Every feature owns its own spec — written in markdown at
`features/<feature>/spec/*.md`, linked to source code and tests via
[tracey](https://github.com/bearcove/tracey) annotations. `cargo xtask
ci` (which runs `tracey query validate`) breaks the build if any rule
is missing an implementation, missing a test, or pointing at a stale
spec ID.

## Layout

```
features/
  example/
    spec/
      repo.md            # ExampleRepo contract rules (r[repo.create.id], …)
    example-memory/      # impl referenced by `// r[impl repo.*]`
    tests/native/        # verification referenced by `// r[verify repo.*]`
```

A new feature `foo` follows the same shape:

```
features/
  foo/
    spec/
      service.md         # FooService contract rules
    foo-proto/
    foo-memory/          # impl annotated `// r[impl foo.service.*]`
    tests/native/        # tests annotated `// r[verify foo.service.*]`
```

## Wiring tracey

Each feature adds one block to `.config/tracey/config.styx`:

```styx
specs (
    {
        name example
        include (features/example/spec/**/*.md)
        impls (
            {
                name rust
                include (features/example/**/*.rs)
                test_include (features/example/tests/**/*.rs)
            }
        )
    }
    # ← copy this block, swap "example" → "foo" for each new feature.
)
```

`tracey query status` then shows a coverage row per feature.

## Writing a rule

In the spec markdown:

```markdown
r[repo.create.id]
A `create` call MUST generate a new UUID for the row's primary key and
return the materialized record with `created_at` populated from the
server clock.
```

In the implementation:

```rust
// r[impl repo.create.id]
async fn create(&self, input: ExampleCreate) -> Result<Example, RepoError> {
    // ...
}
```

In the test:

```rust
// r[verify repo.create.id]
#[tokio::test]
async fn create_returns_populated_record() {
    // ...
}
```

## Naming convention

Tracey enforces lowercase dot-separated segments — no underscores or
hyphens *inside* segments. Prefer `repo.list.sort.name` over
`repo.list.sort.name_asc`; prefer `repo.delete.missing` over
`repo.delete.not_found`. Use periods to nest concepts.

## Where rules render

- **Wiki**: feature specs land at `Specs/<Feature>/<Page>.md` (the
  sync script picks them up automatically; see
  [Getting started](@/getting-started/_index.md) for the workflow).
- **Tracey dashboard**: `tracey web --open` opens a browser UI with
  per-rule status, jump-to-source navigation, and search.
- **CLI queries**: `tracey query status` / `untested` / `uncovered`
  for terminal use.
