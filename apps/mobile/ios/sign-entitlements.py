#!/usr/bin/env python3
"""The entitlements a phone's build is signed with (deploy-testflight.sh).

    sign-entitlements.py make   <profile entitlements> <app entitlements> <out>
    sign-entitlements.py verify <signed entitlements> <expected>

`make`: the profile's identity (its app id, team, keychain groups, whether
TestFlight may run it) and the app's own entitlements
(apps/mobile/ios/Architect.entitlements), each of those only where the
profile allows it: a key the profile does not have is left out (the App ID
lacks the capability) and said so; a value past what the profile allows
fails. Nothing else of the profile's is taken: Apple's HealthKit profile
offers clinical records (`healthkit.access = [health-records]`) and
background delivery, which the app does not ask for.

`verify`: what the signed app carries (`codesign -d --entitlements :-`) is
exactly what `make` wrote, and has no clinical health records.
"""

import plistlib
import sys

# The profile's own, about the app rather than what it may do.
IDENTITY = (
    "application-identifier",
    "com.apple.developer.team-identifier",
    "keychain-access-groups",
    "get-task-allow",
    "beta-reports-active",
)


def load(path):
    with open(path, "rb") as f:
        return plistlib.load(f)


def allows(granted, wanted):
    """Whether the profile's value `granted` covers the app's `wanted`."""
    if isinstance(wanted, bool):
        return granted is True or not wanted
    if isinstance(wanted, list):
        if not isinstance(granted, list):
            return False
        return all(w in granted or "*" in granted for w in wanted)
    return granted == wanted or granted == "*"


def make(profile_path, app_path, out_path):
    profile = load(profile_path)
    app = load(app_path)
    out = {k: profile[k] for k in IDENTITY if k in profile}
    for key, wanted in app.items():
        if key not in profile:
            print(f"note: the profile has no {key}: this build goes without it", file=sys.stderr)
            continue
        if not allows(profile[key], wanted):
            sys.exit(f"ERROR: {key} = {wanted!r} is more than the profile allows ({profile[key]!r})")
        out[key] = wanted
    left = sorted(k for k in profile if k not in out and k not in IDENTITY)
    if left:
        print(f"note: the profile's {', '.join(left)} not asked for: left out", file=sys.stderr)
    with open(out_path, "wb") as f:
        plistlib.dump(out, f)
    for key in sorted(out):
        print(f"  {key} = {out[key]!r}")


def verify(signed_path, expected_path):
    signed = load(signed_path)
    expected = load(expected_path)
    if "health-records" in signed.get("com.apple.developer.healthkit.access", []):
        sys.exit("ERROR: the signed app asks for clinical health records")
    if signed != expected:
        sys.exit(f"ERROR: the signed entitlements are not the ones made:\n{signed!r}\n!=\n{expected!r}")
    print("entitlements verified: the app's own, within the profile, no clinical records")


if __name__ == "__main__":
    if len(sys.argv) == 5 and sys.argv[1] == "make":
        make(*sys.argv[2:])
    elif len(sys.argv) == 4 and sys.argv[1] == "verify":
        verify(*sys.argv[2:])
    else:
        sys.exit(__doc__)
