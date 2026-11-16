/*
 * The interface between a brain and the Piskvork protocol, as in Petr
 * Lastovicka's pisqpipe skeleton that Pela is written against; frontend.cpp
 * implements the protocol side. Part of Tengen; GPL-3.0 or later.
 */
#ifndef TENGEN_PISQPIPE_H
#define TENGEN_PISQPIPE_H

#include "windows.h"

extern int width, height;      /* the board size */
extern int info_timeout_turn;  /* time for one turn in milliseconds */
extern int info_timeout_match; /* total time for a game */
extern int info_time_left;     /* remaining time for a game */
extern DWORD64 info_max_memory; /* maximum memory in bytes, zero if unlimited */
extern int info_game_type;     /* 0 human opponent, 1 AI, 2 tournament, 3 network tournament */
extern int info_exact5;        /* 0 five or more stones win, 1 exactly five */
extern int info_renju;         /* 0 gomoku, 1 renju */
extern int info_caro;          /* 0 gomoku, 1 caro */
extern int info_continuous;    /* 0 single game, 1 continuous */
extern int terminateAI;        /* brain_turn returns when this is above 0 */
extern unsigned start_time;    /* tick count at the start of the turn */
extern char dataFolder[256];   /* folder for persistent files */

void brain_init();
void brain_restart();
void brain_turn();
void brain_my(int x, int y);
void brain_opponents(int x, int y);
void brain_block(int x, int y);
int brain_takeback(int x, int y);
void brain_end();

extern const char *infotext;

int pipeOut(const char *fmt, ...);
void do_mymove(int x, int y);
void suggest(int x, int y);

#endif
