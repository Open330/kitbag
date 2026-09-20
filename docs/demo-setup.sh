#!/bin/sh
# Builds the machine the README GIF is recorded against.
#
# It is a made-up home in a temp directory: nothing in the recording is anybody's
# actual machine, which is the same rule this repository follows everywhere else.
# The quoting lives here rather than in the tape, because VHS has its own string
# parser and fighting two at once is how a demo ends up recording an error.
set -eu

D="$(mktemp -d)"
mkdir -p "$D/.envs" "$D/.config/kitbag" "$D/.aws"

printf '# scope: personal\nexport NOTES_URL=x\nexport NOTES_TOKEN=y\n'        > "$D/.envs/notes.env"
printf '# scope: work\n# owner: acme\nexport CI_TOKEN=x\nexport CI_URL=y\n'   > "$D/.envs/ci.env"
printf '# scope: shared\n# owner: a friend\nexport PROXY_TOKEN=x\n'           > "$D/.envs/proxy.env"
printf '[profile dev]\nregion=eu-west-1\n'                                    > "$D/.aws/config"
printf 'scopes = ["personal", "work", "shared"]\n'                            > "$D/.config/kitbag/machine.toml"

printf '%s\n' "$D"
