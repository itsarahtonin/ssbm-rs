// Mail to the CPU, which the reference drops.
#pragma once

#include "Common/CommonTypes.h"

namespace DSP::HLE
{
class CMailHandler
{
public:
  void PushMail(u32, bool = false, int = 0) {}
};
}  // namespace DSP::HLE
