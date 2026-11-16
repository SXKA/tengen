"""Adapts a copy of SlowRenju's source for builds outside Windows.

SlowRenju (https://github.com/wind23/SlowRenju, GPL-3.0, by Tianyi Hao)
reads commands from a console in a loop and thinks in a second thread. The
page instead calls piskvork_command() with one command at a time and waits
for the answer, so this turns the loop's body into that function and thinks
in the calling thread; SlowRenju's own timer thread, which ends the search,
stays a thread. It also starts every BOARD from an empty board, as the
protocol says, and fixes the MSVC-only spellings.

Usage: python3 patch.py DIRECTORY

Part of Tengen; GPL-3.0 or later.
"""

import pathlib
import re
import sys


def replace(text: str, old: str, new: str, path: pathlib.Path, count: int = 1) -> str:
    """Replaces `old` exactly `count` times, or fails naming the file.

    Args:
        text: The file's text.
        old: The text to replace.
        new: Its replacement.
        path: The file, for the error message.
        count: How many times `old` must occur.

    Returns:
        The text with the replacements made.
    """
    found = text.count(old)
    if found != count:
        sys.exit(f"{path}: expected {count} of {old!r}, found {found}")
    return text.replace(old, new)


def main() -> None:
    root = pathlib.Path(sys.argv[1])
    for path in root.rglob("*"):
        if path.suffix not in (".cpp", ".h"):
            continue
        text = path.read_text(encoding="latin-1")
        # Include paths with backslashes.
        text = re.sub(
            r'#include\s+"([^"]*\\[^"]*)"',
            lambda m: '#include "%s"' % pathlib.PurePosixPath(m.group(1).replace("\\", "/")).name,
            text,
        )
        # The size arguments that MSVC's sscanf_s takes after %s buffers.
        text = re.sub(r"\b(com|key)\s*,\s*9\d\d\b", r"\1", text)
        text = text.replace("sscanf_s(", "sscanf(")
        # A map's allocator holds pairs with const keys; MSVC let this pass.
        text = text.replace("allocator<pair<wstring,int>>", "allocator<pair<const wstring,int>>")
        path.write_text(text, encoding="latin-1")

    main_cpp = root / "Common" / "main.cpp"
    text = main_cpp.read_text(encoding="latin-1")
    # One command per call: the loop ends when the command's lines do.
    text = replace(
        text,
        "int main(int argc, char *argv[])\n{",
        "static std::istringstream feed;\n"
        "#define cin feed\n"
        "static int slowrenju_main(int argc, char *argv[])\n{\n"
        "\tstatic bool started = false;\n"
        "\tif(!started)\n\t{\n\tstarted = true;",
        main_cpp,
    )
    text = replace(
        text,
        '\tprintf_s("MESSAGE SlowRenju %s is ready.\\n", version);\n\tfflush(stdout);\n',
        '\tprintf_s("MESSAGE SlowRenju %s is ready.\\n", version);\n\tfflush(stdout);\n\t}\n',
        main_cpp,
    )
    text = replace(text, "\tint self=-1;\n\twhile(true)\n\t{\n\t\tcin.getline(s,999);",
                   "\tstatic int self=-1;\n\twhile(cin.getline(s,999))\n\t{", main_cpp)
    text = replace(text, "_beginthread(findmove,0,NULL);", "findmove(NULL);", main_cpp, count=3)
    # BOARD is a whole new position, of up to N by N stones.
    text = replace(text, "\t\t\t\tint moveflist[225];", "\t\t\t\tint moveflist[N*N];\n\t\t\t\tmoven=0;", main_cpp)
    text += """
#undef cin

extern "C" void piskvork_command(const char *text)
{
\tfeed.clear();
\tfeed.str(std::string(text) + "\\n");
\tchar name[] = "slowrenju";
\tchar *argv[] = {name, nullptr};
\tslowrenju_main(1, argv);
}
"""
    main_cpp.write_text(text, encoding="latin-1")


if __name__ == "__main__":
    main()
