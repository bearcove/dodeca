#!/bin/bash
exec 2>/tmp/pikru-mcp-debug.log
echo "$(date): Starting pikru-mcp" >&2
echo "PWD: $PWD" >&2
echo "PIKRU_ROOT: $PIKRU_ROOT" >&2
env | grep -i pikru >&2
exec /Users/amos/.cargo/bin/pikru-mcp
