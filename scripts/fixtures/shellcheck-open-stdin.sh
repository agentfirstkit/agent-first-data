#!/bin/bash

# A static-analysis fixture, never executed. The source directive is the whole
# point: under `-x` ShellCheck opens the named path, and this repo's own
# scripts name `/dev/stdin` to load the Bash kit that `afdata shell bash`
# generates at runtime. An analyzer that inherited a terminal's stdin would
# block here forever.
# shellcheck source=/dev/stdin
source /dev/stdin
