#!/bin/bash
# Perf wrapper for nextest - captures CPU samples with stack traces
exec perf record -F 99 -g --call-graph fp -o "perf-$$.data" -- "$@"
