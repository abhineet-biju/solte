#!/bin/sh
# cargo-dist 0.33 does not add --locked to its Cargo invocations.
case "$1" in
  build|metadata) exec cargo "$@" --locked ;;
  *) exec cargo "$@" ;;
esac
