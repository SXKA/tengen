/*
 * The parts of <windows.h> that Pela and SlowRenju use, for builds
 * elsewhere (WebAssembly, and Linux for tests). Part of Tengen; GPL-3.0 or
 * later.
 */
#ifndef TENGEN_COMPAT_WINDOWS_H
#define TENGEN_COMPAT_WINDOWS_H

// Standard headers first: they cannot come after the min and max macros
// below, which <windows.h> defines too.
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <iostream>
#include <mutex>
#include <sstream>
#include <string>
#include <strings.h>
#include <thread>
#include <unistd.h>
#include <unordered_map>
#include <vector>

#define __int64 long long

typedef uint32_t DWORD;
typedef uint64_t DWORD64;
typedef int BOOL;
typedef void *PVOID;
typedef void *LPVOID;
typedef void *HANDLE;
#define WINAPI

/** Milliseconds since an arbitrary start, as GetTickCount counts them. */
inline DWORD GetTickCount()
{
    using namespace std::chrono;
    return (DWORD)duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count();
}

/*
 * clock() in milliseconds, as on Windows, where CLOCKS_PER_SEC is 1000 and
 * the code compares clock() with millisecond budgets. Elsewhere it counts
 * processor time in microseconds.
 */
#define clock() ((clock_t)GetTickCount())

inline void Sleep(DWORD ms)
{
    std::this_thread::sleep_for(std::chrono::milliseconds(ms));
}

/** No program file to look beside: config files are not read. */
inline DWORD GetModuleFileName(void *, char *name, DWORD size)
{
    if (size > 0) name[0] = '\0';
    return 0;
}

#define UInt32x32To64(a, b) ((uint64_t)(uint32_t)(a) * (uint64_t)(uint32_t)(b))

// As <windows.h> has them; standard headers must come before this one.
#ifndef min
#define min(a, b) (((a) < (b)) ? (a) : (b))
#endif
#ifndef max
#define max(a, b) (((a) > (b)) ? (a) : (b))
#endif

#define _stricmp strcasecmp
#define _strnicmp strncasecmp
#define _atoi64 atoll
#define printf_s printf

inline int strcat_s(char *dest, size_t size, const char *src)
{
    size_t used = strlen(dest);
    if (used + 1 < size) strncat(dest, src, size - used - 1);
    return 0;
}
#define sprintf_s snprintf

#endif
