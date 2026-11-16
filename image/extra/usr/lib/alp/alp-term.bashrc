# shellcheck shell=bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fixed settings of the Terminal's shell (docs/plans/live-terminal.md).
#
# The command log (T4) is a record kept in good faith, NOT a security
# control: `exec sh`, or any program with a shell of its own, gets around
# it, and a command line can hold a secret. The protection is elsewhere:
# the unprivileged user, the firewall rule (T1) and the unit's sandbox (T3).

# No history file: nothing of the session is kept but the journal lines.
HISTFILE=
HISTSIZE=1000
HISTCONTROL=
# Rule 5: no input for 10 minutes at the prompt ends the session (ttyd
# --once then ends the unit).
readonly TMOUT=600

__alp_seen=
__alp_log() {
    local line n cmd
    line=$(HISTTIMEFORMAT='' builtin history 1) || return 0
    read -r n cmd <<<"$line"
    [[ -n $n && $n != "$__alp_seen" ]] || return 0
    __alp_seen=$n
    logger -t alp-terminal -- "command: $cmd"
}
readonly -f __alp_log
readonly PROMPT_COMMAND=__alp_log

# T6: the system log as it was when the session started, written by root
# from fixed sources (alpctl live term-logs); nothing typed here reaches
# journalctl with privileges.
logs() { more /run/alp-terminal/logs.txt; }

PS1='alp-term \w \$ '
cd /tmp || true
echo "Commands you run here are kept in the system log."
echo "Type 'logs' to read the system log (as it was when this session started)."
