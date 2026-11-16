/*
 * A console front for piskvork_command(), so a brain built for the page can
 * be tested natively over the Piskvork protocol: BOARD and its lines go in
 * as one command. Part of Tengen; GPL-3.0 or later.
 */
#include <iostream>
#include <string>
#include <strings.h>

extern "C" void piskvork_command(const char *text);

int main()
{
    std::string line;
    while (std::getline(std::cin, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        std::string command = line;
        if (strncasecmp(line.c_str(), "BOARD", 5) == 0) {
            while (std::getline(std::cin, line)) {
                if (!line.empty() && line.back() == '\r') line.pop_back();
                command += "\n" + line;
                if (strcasecmp(line.c_str(), "DONE") == 0) break;
            }
        }
        piskvork_command(command.c_str());
        if (strncasecmp(command.c_str(), "END", 3) == 0) break;
    }
}
