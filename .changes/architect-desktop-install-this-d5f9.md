---
"architect-desktop" = "minor"
---

architect-desktop install: this build as the installed app, from a checkout. On a Mac bundled, made into a disk image, signed with the Developer ID, notarized and stapled as a release is (ad hoc and unnotarized without the identity; --no-notarize), in /Applications or ~/Applications (--user), the disk image kept; on Linux the binary and a desktop entry under ~/.local. The command is linked onto PATH as ~/.local/bin/architect.
