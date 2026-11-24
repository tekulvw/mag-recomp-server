#!/usr/bin/env bash
# shellcheck disable=SC2086 # $checks is a word list
# Choose which jobs of the public source checks a change needs.
#
#   select-checks.sh changed < paths   repository-relative changed paths, one per line
#   select-checks.sh list NAME...      exactly the named checks; "all" selects every check
#
# Prints one NAME=true|false line per check, ready for $GITHUB_OUTPUT. Any doubt
# selects more: workflow and CI script changes, or an empty or unreadable path
# list, select every check. Bash 3.2 compatible so it also runs on macOS.
set -euo pipefail

checks='boundary python gameplay horizon registry portal chart release'

select_check() {
  local name
  for name in "$@"; do
    case " $checks " in
      *" $name "*) printf -v "selected_$name" true ;;
      *) printf 'select-checks: unknown check %s\n' "$name" >&2; exit 2 ;;
    esac
  done
}

for name in $checks; do printf -v "selected_$name" false; done

case "${1:-}" in
  changed)
    # The publication boundary (public-tree scan) is cheap and covers every file.
    select_check boundary
    seen=false
    while IFS= read -r path || [ -n "$path" ]; do
      [ -n "$path" ] || continue
      seen=true
      case "$path" in
        .github/*) select_check $checks ;;
      esac
      # Python unit tests (tests/*.py) and the tools and files they read.
      case "$path" in
        tests/*|tools/*|charts/*|docs/main-images-values.yaml|portal/tests/ticket-vectors.json|horizon/Tests.MagPlugin/portal-ticket-vectors.json)
          select_check python ;;
      esac
      case "$path" in
        gameplay/*) select_check gameplay registry ;;
        horizon/*) select_check horizon registry ;;
        portal/*) select_check portal ;;
        # The configured-world check builds Medius against real gameplay workers.
        tests/Dockerfile.world-registry|tests/registry_fixture.rs) select_check registry ;;
      esac
      case "$path" in
        charts/*|tests/test_helm_chart.py|tests/test_data_bundle.py|tools/data-bundle.py)
          select_check chart ;;
      esac
      # Release Please routing reads the config, manifest and every component version file.
      case "$path" in
        release-please-config.json|.release-please-manifest.json|tests/release-routing.cjs|tests/pr-title.cjs|tools/check-pr-title.cjs|horizon/version.txt|gameplay/Cargo.toml|gameplay/Cargo.lock|portal/Cargo.toml|portal/Cargo.lock|charts/mag-recomp-server/Chart.yaml)
          select_check release ;;
      esac
    done
    [ "$seen" = true ] || select_check $checks
    ;;
  list)
    shift
    for name in "$@"; do
      if [ "$name" = all ]; then select_check $checks; else select_check "$name"; fi
    done
    ;;
  *)
    printf 'usage: select-checks.sh changed < paths | list NAME...\n' >&2
    exit 2
    ;;
esac

for name in $checks; do
  var="selected_$name"
  printf '%s=%s\n' "$name" "${!var}"
done
