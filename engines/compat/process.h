/*
 * _beginthread and _endthread from <process.h> on std::thread, for
 * SlowRenju. Part of Tengen; GPL-3.0 or later.
 */
#ifndef TENGEN_COMPAT_PROCESS_H
#define TENGEN_COMPAT_PROCESS_H

#include <thread>

inline void *_beginthread(void (*start)(void *), unsigned, void *arg)
{
    std::thread(start, arg).detach();
    return nullptr;
}

/** The thread function returns right after, which ends the thread. */
inline void _endthread() {}

#endif
