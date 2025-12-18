#!/bin/bash
# Systemd-run wrapper for nextest
# Runs tests through systemd-run with 4GB memory limit to avoid OOM issues
exec systemd-run --user --scope -p MemoryMax=4G "$@"
