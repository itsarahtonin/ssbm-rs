// SPDX-License-Identifier: GPL-3.0-or-later
// Included before each Dolphin SDK source for clang: what MWCC reads there that clang cannot.

// Assembly functions are declared with `asm`; their bodies are left out before parsing.
#define asm

// Variables MWCC places at fixed addresses (dolphin/os.h), as the memory they stand for.
#define __OSPhysicalMemSize (*(u32*) 0x80000028)
#define __OSTVMode (*(volatile int*) 0x800000CC)
#define __gUnkThread1 (*(OSThread**) 0x800000D8)
#define __OSActiveThreadQueue (*(OSThreadQueue*) 0x800000DC)
#define __gCurrentThread (*(OSThread**) 0x800000E4)
#define __OSSimulatedMemSize (*(u32*) 0x800000F0)
#define __EXIProbeStartTime ((int*) 0x800030C0)

// Stands in for an inline `asm { }` block, so the function holding one is not translated.
void __c2rs_inline_asm(void);
