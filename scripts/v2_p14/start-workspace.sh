#!/bin/sh
set -eu
if [ "$#" -lt 1 ]; then
  printf 'Usage: %s AUTHORIZED_PROJECT_DIRECTORY [factory start options]\n' "$0" >&2
  exit 2
fi
project_root=$(cd -- "$1" && pwd -P)
shift
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
exec "$script_root/factory" start --workspace --no-default-project --allow-project-root "$project_root" "$@"
