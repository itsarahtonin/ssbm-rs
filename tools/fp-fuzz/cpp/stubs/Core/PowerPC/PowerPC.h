// The parts of Dolphin's PowerPC.h and PowerPC.cpp that the float instructions use.
#pragma once

#include "Common/CommonTypes.h"
#include "Common/FloatUtils.h"
#include "Core/PowerPC/ConditionRegister.h"
#include "Core/PowerPC/Gekko.h"

namespace PowerPC
{
struct PairedSingle
{
  u64 PS0AsU64() const { return ps0; }
  u64 PS1AsU64() const { return ps1; }
  u32 PS0AsU32() const { return static_cast<u32>(ps0); }
  u32 PS1AsU32() const { return static_cast<u32>(ps1); }
  double PS0AsDouble() const { return std::bit_cast<double>(ps0); }
  double PS1AsDouble() const { return std::bit_cast<double>(ps1); }
  void SetPS0(u64 value) { ps0 = value; }
  void SetPS0(double value) { ps0 = std::bit_cast<u64>(value); }
  void SetPS1(u64 value) { ps1 = value; }
  void SetPS1(double value) { ps1 = std::bit_cast<u64>(value); }
  void SetBoth(u64 lhs, u64 rhs)
  {
    SetPS0(lhs);
    SetPS1(rhs);
  }
  void SetBoth(double lhs, double rhs)
  {
    SetPS0(lhs);
    SetPS1(rhs);
  }
  void Fill(u64 value) { SetBoth(value, value); }
  void Fill(double value) { SetBoth(value, value); }

  u64 ps0 = 0;
  u64 ps1 = 0;
};

struct PowerPCState
{
  PairedSingle ps[32];
  ConditionRegister cr{};
  UReg_MSR msr;
  UReg_FPSCR fpscr;
  u32 Exceptions = 0;
  u32 spr[1024]{};

  void UpdateCR1()
  {
    cr.SetField(1, (fpscr.FX << 3) | (fpscr.FEX << 2) | (fpscr.VX << 1) | fpscr.OX);
  }
  void UpdateFPRFDouble(double dvalue) { fpscr.FPRF = Common::ClassifyDouble(dvalue); }
  void UpdateFPRFSingle(float fvalue) { fpscr.FPRF = Common::ClassifyFloat(fvalue); }
};
}  // namespace PowerPC
