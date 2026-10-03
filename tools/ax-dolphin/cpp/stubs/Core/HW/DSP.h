// ARAM, through the bus the shim is given.
#pragma once

#include "Common/CommonTypes.h"
#include "Core/HW/Memmap.h"

namespace DSP
{
class DSPManager
{
public:
  u8 ReadARAM(u32 address) const
  {
    auto* bus = AxDolphin::CurrentBus();
    return bus->aram(bus->user, address & 0x00FFFFFF);
  }
  void WriteARAM(u8, u32) {}
};
}  // namespace DSP
