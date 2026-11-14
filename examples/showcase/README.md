# The extension showcase's examples

Small example extensions, one per capability or group of capabilities
from architect#710, each written the way a third party would: a folder
with an `extension.toml`, loaded from a registry or as a development
copy. Each says, in its manifest's description and its note in the
showcase vault (`apps/ui/showcase-vault`), which Obsidian plugins or apps
it stands in for.

`just showcase` runs the app on a scratch copy of the vault with every
folder here loaded as a development copy: edit one and save, and it
reloads. The matrix of what runs where is
`docs/content/guide/showcase.md`.
