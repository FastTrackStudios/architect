+++
title = "Forms (architect-form)"
description = "Typed, validated form state for Dioxus — the effect-form analog, building on architect-atom and submitting through vox."
weight = 56
+++

`architect-form` (`features/form`, re-exported as the `architect::form`
module under the `form` feature — `architect::form::Field`, `architect::form::TextField`, …) is the form-shaped sibling of
[`architect-atom`](@/architecture/optimistic.md), modelled on
[effect-form](https://github.com/lucas-barake/effect-form). It's the
**controlled** improvement over the baseline Dioxus form.

## The baseline it improves on

Dioxus's built-in form is *uncontrolled*: inputs carry `name` attributes
and you pull the values off the `FormEvent` on submit —

```rust
form {
    onsubmit: move |evt: FormEvent| {
        evt.prevent_default();
        let values: LoginForm = evt.parsed_values().unwrap();   // name-keyed deserialize
        // …submit values…
    },
    input { name: "username" }
    input { name: "password" }
}
```

That's fine for a throwaway form, but there's no per-field validation, no
error/touched/dirty state, no "disable submit while invalid", and (in the
fullstack docs) it hands off to a `#[post]` **server function** —
[which architect forbids](@/architecture/idioms.md): all I/O goes through
vox. `architect-form` keeps `evt.parsed_values()`'s spirit (it's a core
`FormEvent` method, fine in CSR) but makes the form *controlled* and
submits through the vox client.

## Forms are derived from the entity

This is the part effect-form can't do: Rust's "schema" is the type
itself, and the Entity derive already owns it. Add `form` to the derive
and the typed form layer is emitted over the Create/Update payloads:

```rust
#[architect(table_name = "examples", repo, store, form)]
pub struct Example {
    #[architect(filterable, sortable, fulltext)]
    pub name: String,                                  // form: required
    #[architect(filterable, fulltext, form(optional))]
    pub description: String,                           // form: optional
    …
}
```

generates (gated on the proto crate's `form = ["architect/form"]`):

- **`ExampleCreateFields`** / `use_example_create_fields()` — one
  validated `Field` per Create-payload field. `String` fields are
  required (Title-Case label, override with `form(label = "…")`) unless
  marked `form(optional)`; any other type validates through
  `validate::parse::<T>` (`FromStr`).
- **`ExampleUpdateFields`** / `use_example_update_fields(&example)` —
  the same fields, **seeded from the current row**.
- **`submit() -> Option<ExampleCreate>`** (resp. `ExampleUpdate`) —
  validates every field (revealing errors) and returns the **typed wire
  payload**, the exact struct the derived mutations take. Plus
  `is_dirty()` / `reset()`.

With the generic `TextField` component (label + controlled input +
inline error over any `Field<T>`), a whole create form is layout only:

```rust
let fields = use_example_create_fields();
rsx! {
    form {
        onsubmit: move |evt| {
            evt.prevent_default();
            if let Some(input) = fields.submit() {     // -> ExampleCreate
                on_submit.call(input);                  // → mutations.create(input)
                fields.reset();
            }
        },
        TextField { field: fields.name, label: "Name" }
        TextField { field: fields.description, label: "Description", placeholder: "optional" }
        button { class: "btn primary", r#type: "submit", "Add example" }
    }
}
```

The pipeline is typed end-to-end: **entity → form fields → validated
payload → optimistic mutation → reconcile** — one source of truth, zero
hand-written field plumbing.

## Fields (the primitive underneath)

A [`Field<T>`](https://docs.rs/architect-form) owns a string-encoded value
plus a **parser that validates and decodes** into a typed `T` — the role
effect-form gives an Effect `Schema`. It tracks `error`, `touched`, and
`dirty`, and re-validates live once the field has been touched:

```rust
use architect::form::{use_field, validate};

let name = use_field("", validate::required("Name"));   // -> Field<String>
let desc = use_field("", validate::optional());
let age  = use_field("", |s: &str| s.parse::<u8>().map_err(|_| "must be a number".into()));
```

```rust
let name_value = name.value();
let name_error = name.error();   // Some(..) only after touch / submit attempt
rsx! {
    input {
        value: "{name_value}",
        oninput: move |e| name.set(e.value()),     // live re-validate once touched
        onblur:  move |_| name.blur(),             // mark touched + validate
    }
    if let Some(err) = name_error {
        span { class: "field-error", "{err}" }
    }
}
```

Validator combinators live in [`validate`](https://docs.rs/architect-form):
`required`, `optional`, `min_length`, `max_length`, `email`, and `and` to
chain them. Any `Fn(&str) -> Result<T, String>` works, so a field can
decode to any type (`u8`, a domain enum, …).

Hand-write fields only for forms that don't mirror a payload (login,
search, settings); payload-shaped forms come from the derive. The worked
references: derived —
`examples/app/features/example/example-ui/src/components/forms/`; the
primitive — the same files, since the components are now just `TextField`
layout over the generated fields.

## Field components

The rendering half ships ready-made — label + control + inline error over
any `Field`:

- **`TextField`** / **`TextArea`** — controlled text inputs (generic over
  the decoded type; a `Field<u32>` renders like a `Field<String>`, the
  parser is the difference).
- **`SelectField`** — `<select>` over `(value, display)` options; pair
  with `validate::parse::<YourEnum>` for typed enums.
- **`Checkbox`** — over a `Field<bool>` (seed with `"false"`,
  `validate::parse::<bool>`).

Apps with a design system write their own equivalents — the `Field`
surface (`value` / `set` / `blur` / `error`) is the contract.

## Submitting

There is one way to submit: the (usually derived) fields produce the
typed wire payload, and the derived optimistic mutations take it —

```rust
if let Some(input) = fields.submit() {     // -> ExampleCreate, errors revealed
    mutations.create(input);               // optimistic row + reconcile/rollback
}
```

Pending/error state lives on the mutation handle (`is_pending`,
`create_error`), failures report to the app's `Notifications`, and the
row appears instantly — there's no separate form-submit machinery.

> Enforced: validator combinators are unit-tested in
> `features/form`; the example's create form is exercised by
> `example-ui`'s `tests/components.rs`.
