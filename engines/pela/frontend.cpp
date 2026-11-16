/*
 * The Piskvork protocol for brains written against the pisqpipe skeleton,
 * without threads or blocking input, for WebAssembly: the page calls
 * piskvork_command() with one command at a time (BOARD with its lines and
 * DONE), and a turn is thought over before the call returns. Answers go to
 * standard output, one line each.
 *
 * Part of Tengen; GPL-3.0 or later.
 */
#include <cctype>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sstream>
#include <string>

#include "pisqpipe.h"

int width, height;
int info_timeout_turn = 30000;
int info_timeout_match = 1000000000;
int info_time_left = 1000000000;
DWORD64 info_max_memory = 0;
int info_game_type = 1;
int info_exact5 = 0;
int info_renju = 0;
int info_caro = 0;
int info_continuous = 0;
int terminateAI;
unsigned start_time;
char dataFolder[256];

/** Output is dropped while set: BOARD restarts the brain without its OK. */
static bool quiet = false;

int pipeOut(const char *fmt, ...)
{
    if (quiet) return 0;
    va_list va;
    va_start(va, fmt);
    int n = vprintf(fmt, va);
    va_end(va);
    putchar('\n');
    fflush(stdout);
    return n;
}

void suggest(int x, int y) { pipeOut("SUGGEST %d,%d", x, y); }

void do_mymove(int x, int y)
{
    brain_my(x, y);
    pipeOut("%d,%d", x, y);
}

/** The text after `command` and spaces, if `input` starts with it. */
static const char *param_of(const char *command, const char *input)
{
    size_t n = strlen(command);
    if (strncasecmp(command, input, n) != 0) return nullptr;
    input += n;
    if (*input != '\0' && !isspace((unsigned char)*input)) return nullptr;
    while (isspace((unsigned char)*input)) input++;
    return input;
}

static bool parse_coord(const char *text, int *x, int *y)
{
    return sscanf(text, "%d,%d", x, y) == 2 && *x >= 0 && *y >= 0 && *x < width && *y < height;
}

static void start()
{
    start_time = GetTickCount();
    if (!width) {
        width = height = 20;
        brain_init();
    }
}

static void turn()
{
    terminateAI = 0;
    start_time = GetTickCount();
    brain_turn();
}

static void info(const char *param)
{
    const char *v;
    if ((v = param_of("max_memory", param))) info_max_memory = atoll(v);
    if ((v = param_of("timeout_match", param))) info_timeout_match = atoi(v);
    if ((v = param_of("timeout_turn", param))) info_timeout_turn = atoi(v);
    if ((v = param_of("time_left", param))) info_time_left = atoi(v);
    if ((v = param_of("game_type", param))) info_game_type = atoi(v);
    if ((v = param_of("rule", param))) {
        int e = atoi(v);
        info_exact5 = e & 1;
        info_continuous = (e >> 1) & 1;
        info_renju = (e >> 2) & 1;
        info_caro = (e >> 3) & 1;
    }
    if ((v = param_of("folder", param))) strncpy(dataFolder, v, sizeof(dataFolder) - 1);
}

extern "C" void piskvork_command(const char *text)
{
    std::istringstream lines(text);
    std::string line;
    if (!std::getline(lines, line)) return;
    if (!line.empty() && line.back() == '\r') line.pop_back();
    const char *cmd = line.c_str();
    const char *param;
    int x, y;

    if ((param = param_of("info", cmd))) {
        info(param);
    } else if ((param = param_of("start", cmd))) {
        if (sscanf(param, "%d", &width) != 1 || width < 5) {
            width = 0;
            pipeOut("ERROR bad START parameter");
        } else {
            height = width;
            start_time = GetTickCount();
            brain_init();
        }
    } else if ((param = param_of("rectstart", cmd))) {
        if (sscanf(param, "%d ,%d", &width, &height) != 2 || width < 5 || height < 5) {
            width = height = 0;
            pipeOut("ERROR bad RECTSTART parameters");
        } else {
            start_time = GetTickCount();
            brain_init();
        }
    } else if ((param = param_of("restart", cmd))) {
        start();
        brain_restart();
    } else if ((param = param_of("turn", cmd))) {
        start();
        if (!parse_coord(param, &x, &y)) {
            pipeOut("ERROR bad coordinates");
        } else {
            brain_opponents(x, y);
            turn();
        }
    } else if ((param = param_of("begin", cmd))) {
        start();
        turn();
    } else if ((param = param_of("board", cmd))) {
        start();
        // BOARD sets a whole new position, so the board starts empty.
        quiet = true;
        brain_restart();
        quiet = false;
        while (std::getline(lines, line)) {
            if (!line.empty() && line.back() == '\r') line.pop_back();
            int who = 0;
            if (sscanf(line.c_str(), "%d,%d,%d", &x, &y, &who) != 3 || x < 0 || y < 0 || x >= width || y >= height) {
                if (strcasecmp(line.c_str(), "done") != 0) pipeOut("ERROR x,y,who or DONE expected after BOARD");
                break;
            }
            if (who == 1) brain_my(x, y);
            else if (who == 2) brain_opponents(x, y);
            else if (who == 3) brain_block(x, y);
        }
        turn();
    } else if ((param = param_of("takeback", cmd))) {
        start();
        const char *answer = "ERROR bad coordinates";
        if (parse_coord(param, &x, &y)) {
            int e = brain_takeback(x, y);
            if (e == 0) answer = "OK";
            else if (e == 1) answer = "UNKNOWN";
        }
        pipeOut("%s", answer);
    } else if ((param = param_of("about", cmd))) {
        pipeOut("%s", infotext);
    } else if ((param = param_of("end", cmd))) {
        brain_end();
    } else {
        pipeOut("UNKNOWN command");
    }
}
