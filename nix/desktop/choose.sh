#!/usr/bin/env bash
# Which AppImage the architect-desktop launcher runs (#523): the newer of the
# store's and the one the app's updater downloaded.
#
#   choose.sh <store-version> <store-image> <update-dir>
#
# Prints the path of the image to run. The downloaded image is
# <update-dir>/Architect-linux-x86_64.AppImage and its version is the
# trimmed content of <update-dir>/version: X.Y.Z or X.Y.Z-nightly.YYYYMMDD.
# Versions compare by component; a release beats a nightly of the same
# X.Y.Z. A missing, empty or unparsable version, or a missing or
# non-executable image, means the store's. The store's wins a tie.
set -u

store_version="$1"
store_image="$2"
update_dir="$3"

# "X Y Z R D": R is 1 for a release, 0 for a nightly; D the nightly's date.
key() {
    if [[ "$1" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)(-nightly\.([0-9]{8}))?$ ]]; then
        local r=1 d=0
        if [ -n "${BASH_REMATCH[4]}" ]; then r=0 d="${BASH_REMATCH[5]}"; fi
        echo "$((10#${BASH_REMATCH[1]})) $((10#${BASH_REMATCH[2]})) $((10#${BASH_REMATCH[3]})) $r $((10#$d))"
    else
        return 1
    fi
}

# Whether version $1 is strictly newer than version $2 (both parse).
newer() {
    local a b i
    read -ra a <<<"$(key "$1")"
    read -ra b <<<"$(key "$2")"
    for i in 0 1 2 3 4; do
        if ((a[i] > b[i])); then return 0; fi
        if ((a[i] < b[i])); then return 1; fi
    done
    return 1
}

image="$update_dir/Architect-linux-x86_64.AppImage"
version=""
if [ -r "$update_dir/version" ]; then
    version="$(tr -d '[:space:]' <"$update_dir/version")"
fi

if [ -f "$image" ] && [ -x "$image" ] && key "$version" >/dev/null \
    && { ! key "$store_version" >/dev/null || newer "$version" "$store_version"; }; then
    echo "$image"
else
    echo "$store_image"
fi
