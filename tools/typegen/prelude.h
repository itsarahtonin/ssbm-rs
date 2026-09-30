// SPDX-License-Identifier: GPL-3.0-or-later
// Included before each unit for clang: what MWCC knows without a declaration.

// Stands in for an inline `asm { }` block, so the function holding one is ported from its
// machine code.
void __c2rs_inline_asm(void);

// MWCC's intrinsics, as MetroTRK/intrinsics.h declares them.
int __rlwinm(int, int, int, int);
int __rlwimi(int, int, int, int, int);
int __cntlzw(unsigned int);
