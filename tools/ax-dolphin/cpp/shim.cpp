// SPDX-License-Identifier: GPL-3.0-or-later
// Builds Dolphin's AX microcode HLE (GPL-2.0-or-later) from its source as a reference, behind a
// C interface: a command list runs against a bus the caller gives.

#include <memory>

#include "Common/Logging/Log.h"
#include "Common/MathUtil.h"

#include "Core/DSP/DSPAccelerator.cpp"
#include "Core/HW/DSPHLE/UCodes/AX.cpp"

namespace
{
// AX's own command list handling, without the mail around it.
struct Harness final : DSP::HLE::AXUCode
{
  Harness(DSP::HLE::DSPHLE* hle, u32 crc) : AXUCode(hle, crc) {}
  void Run(u32 addr, u16 size)
  {
    CopyCmdList(addr, size);
    HandleCommandList();
  }
};

struct Instance
{
  DSP::HLE::DSPHLE hle;
  std::unique_ptr<Harness> ax;
};
}  // namespace

extern "C" void* ax_dolphin_new(u32 crc, const u8* coefs, size_t len)
{
  AxDolphin::Coefs().assign(coefs, coefs + len);
  auto* inst = new Instance;
  inst->ax = std::make_unique<Harness>(&inst->hle, crc);
  inst->ax->Initialize();
  return inst;
}

extern "C" void ax_dolphin_run(void* handle, AxDolphin::Bus* bus, u32 addr, u16 size)
{
  AxDolphin::CurrentBus() = bus;
  static_cast<Instance*>(handle)->ax->Run(addr, size);
  AxDolphin::CurrentBus() = nullptr;
}

extern "C" void ax_dolphin_free(void* handle)
{
  delete static_cast<Instance*>(handle);
}
