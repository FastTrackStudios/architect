# Showcase: install on open

This vault names the extensions it needs in `.architect/extensions.toml`
and carries one in `.architect/extensions/`. Opening it shows what is
missing, with **Install all**.

The carried one, Tiny Theme Tokens, is refused: it is signed by the
showcase registry's key, and a vault's archive must be signed by the org
key or by the key its installed copy is signed by.
