#!/bin/bash
########################################################################
# Converts pikchr-format text files into a form usable by the
# Fossil SCM's pikchrshow page's "example scripts" JS code.
#
# Usage: $0 [options] [file1.pikchr [...fileN.pikchr]]
#
# Options:
#
#  -o outfile, defaulting to /dev/stdout
#
# Its list of files defaults to $(ls -1 *.pikchr | sort).
########################################################################

function die(){
    local rc=$1
    shift
    echo "$@" 1>&2
    exit $rc
}

optOutfile=/dev/stdout
scriptList=()
while [[ x != "x$1" ]]; do
    case "$1" in
        -o) shift
            optOutfile="$1"
            shift
            [[ x = "x${optOutfile}" ]] && die 1 "Missing filename for -o arg."
            ;;
        *) scriptList+=("$1")
           shift
    esac
done

[[ 0 = ${#scriptList[@]} ]] && {
    scriptList=( $(ls -1 *.pikchr | sort) )
    [[ 0 = ${#scriptList[@]} ]] && die 1 "Cannot find any *.pikchr files."
}

########################################################################
# Optional *brief* user-friendly descriptive name of test files, in
# the form desc_TESTNAME="name", where TESTNAME is the base filename
# part of an input file. If none is set, the file is grepped for
# a line with:
#
#  demo label: ...
#
# and if set, that is used. The default friendly name is that base
# filename. These names are the ones shown in pikchrshow's example
# script selection list.
#desc_objects="Core object types"
#desc_swimlane="Swimlanes"
#desc_headings01="Cardinal headings"
########################################################################

#echo "scriptList=${scriptList[@]}"
#echo optOutfile=${optOutfile}
########################################################################
# Output 1 JS object per file, comma-separated:
#
#  {name: "filename or desc_BASENAME value",
#   code: `file content`}
#
# Note that the output is intended for embedding in an array but does
# not emit the [...] part itself because of how its output is used.
{
    n=0 # object count
    for f in ${scriptList[@]}; do
        [[ -f "$f" ]] || die $? "Missing file: $f"
        fb=${f%%.pikchr}
        fb=${fb##*/}
        descVar=desc_${fb}
        desc=${!descVar}
        if [[ x = "x${desc}" ]]; then
            desc=$(awk -F: '/demo label: */{gsub(/^ /, "", $2); print $2}' < "$f")
            if [[ x = "x${desc}" ]]; then
                desc="$fb"
            fi
        fi
        #echo f=${f} fb=${fb} descV=${descV} desc=$desc
        [[ $n -gt 0 ]] && echo -n ","
        echo -n '{name:"'${desc}'",'
        echo -n 'code:`'
        cat $f
        echo -n '`}'
        n=$((n + 1))
    done
    echo
} > "${optOutfile}"
#echo "Done: ${n} file(s) processes. Output is in ${optOutfile}."
