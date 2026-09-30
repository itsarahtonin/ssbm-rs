// An x86-64 host with SSE flush-to-zero, as Dolphin sees it.
#pragma once

struct CPUInfo
{
  bool bFlushToZero = true;
  bool bFMA = true;
};

inline CPUInfo cpu_info;
