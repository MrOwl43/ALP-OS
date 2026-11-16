#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# The version systemd-boot and systemd-sysupdate compare (ADR 0026), from the
# release name in image/extra/usr/lib/alp/release (or the name given).
#
# Releases are numbered in the order they are made: 0.2.6, 0.2.61, 0.2.7.
# Compared number by number, 0.2.61 would be newer than 0.2.7. So the third
# number is read as two digits: 0.2.7 is 0.2.70, 0.2.61 stays 0.2.61, and
# 0.3 is 0.3.00. alpd shows the short form again (update::display).
#
# Usage: tools/update-version.sh [release-name]
set -euo pipefail
here=$(dirname "$(realpath "$0")")
name=${1:-$(awk '{print $NF}' "$here/../image/extra/usr/lib/alp/release")}
if [[ ! $name =~ ^([0-9]+)\.([0-9]+)(\.([0-9]{1,2}))?$ ]]; then
    echo "update-version: '$name' is not a release name like 0.2.7 or 0.2.61" >&2
    exit 1
fi
third=${BASH_REMATCH[4]:-0}
# 0.2.10 would be the same version as 0.2.1, and 0.2.05 would sort before it.
if [[ ${#third} == 2 && ( $third == 0* || $third == *0 ) ]]; then
    echo "update-version: '$name' is not a release name: a two-digit third number cannot start or end with 0" >&2
    exit 1
fi
(( ${#third} == 2 )) || third="${third}0"
echo "${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.${third}"
