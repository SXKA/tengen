#!/bin/sh
# Builds a classic brain for the website, or natively for tests.
#
#   engines/build.sh pela SOURCE OUT [native]
#   engines/build.sh slowrenju SOURCE OUT [native]
#
# SOURCE is a checkout of https://github.com/plastovicka/Piskvork (Pela) or
# https://github.com/wind23/SlowRenju. The WebAssembly build (the default)
# needs emcc and writes OUT/<brain>.js and OUT/<brain>.wasm; the native one
# writes OUT/pbrain-<brain>, which speaks the Piskvork protocol on the
# console.
set -eu
brain=$1
source=$(cd "$2" && pwd)
mkdir -p "$3"
out=$(cd "$3" && pwd)
target=${4:-wasm}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case $brain in
  pela)
    cp "$source"/source/pbrain/*.cpp "$source"/source/pbrain/*.h "$work/"
    # The skeleton's header is replaced by ours.
    sed -i 's#"\.\.\\skelet_CPP\\pisqpipe\.h"#"pisqpipe.h"#' "$work/board.h"
    sources="$work/alfabeta.cpp $work/piskcomp.cpp $work/pisktur.cpp $here/pela/frontend.cpp"
    includes="-I$here/pela -I$here/compat -I$work"
    threads=""
    ;;
  slowrenju)
    cp -r "$source"/AI "$source"/Common "$source"/Headers "$source"/Shape "$source"/Value "$source"/VCF "$work/"
    python3 "$here/slowrenju/patch.py" "$work"
    sources=$(find "$work" -name '*.cpp' ! -path '*/Android/*' | sort | tr '\n' ' ')
    includes="-I$here/compat -I$work/Headers"
    threads=1
    ;;
  *)
    echo "unknown brain $brain" >&2
    exit 2
    ;;
esac

# Old code: no warnings, and g++ needs to be lenient about some of it.
flags="-O2 -std=c++17 -w -DNDEBUG"
if [ "$target" = native ]; then
  cxx=${CXX:-g++}
  case $cxx in *g++*) flags="$flags -fpermissive" ;; esac
  # shellcheck disable=SC2086
  $cxx $flags $includes $sources "$here/native-main.cpp" -o "$out/pbrain-$brain" -pthread
else
  link="-sMODULARIZE=1 -sEXPORT_NAME=Brain -sEXPORTED_FUNCTIONS=_piskvork_command \
    -sEXPORTED_RUNTIME_METHODS=ccall -sALLOW_MEMORY_GROWTH=1 -sINITIAL_MEMORY=134217728 \
    -sSTACK_SIZE=4MB -sENVIRONMENT=web,worker,node"
  if [ -n "$threads" ]; then
    # SlowRenju stops its search from a timer thread.
    link="$link -pthread -sPTHREAD_POOL_SIZE=2"
    flags="$flags -pthread"
  fi
  # shellcheck disable=SC2086
  emcc $flags $includes $sources $link -o "$out/$brain.js"
fi
