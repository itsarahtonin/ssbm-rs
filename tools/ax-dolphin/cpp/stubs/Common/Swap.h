// Byte swaps, as Dolphin's Common/Swap.h has them.
#pragma once

#include "Common/CommonTypes.h"

namespace Common
{
inline u16 swap16(u16 v)
{
  return static_cast<u16>((v >> 8) | (v << 8));
}
inline u32 swap32(u32 v)
{
  return (v >> 24) | ((v >> 8) & 0xFF00) | ((v << 8) & 0xFF0000) | (v << 24);
}
}  // namespace Common
