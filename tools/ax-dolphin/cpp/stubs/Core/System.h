// The system: its memory and DSP.
#pragma once

#include "Core/HW/DSP.h"
#include "Core/HW/Memmap.h"

namespace Core
{
class System
{
public:
  Memory::MemoryManager& GetMemory() { return m_memory; }
  DSP::DSPManager& GetDSP() { return m_dsp; }

private:
  Memory::MemoryManager m_memory;
  DSP::DSPManager m_dsp;
};
}  // namespace Core
