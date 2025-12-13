#!/bin/bash
# Strace wrapper for nextest
# Outputs to strace-$$.log in current directory
exec strace -f -e trace=mmap,munmap,brk,mprotect,execve,write -o "strace-$$.log" "$@"
