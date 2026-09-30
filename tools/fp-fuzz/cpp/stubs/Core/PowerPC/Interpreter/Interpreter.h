// The float instructions of Dolphin's Interpreter class, without the rest of the emulator.
#pragma once

#include "Core/PowerPC/Gekko.h"
#include "Core/PowerPC/PowerPC.h"

class Interpreter
{
public:
  explicit Interpreter(PowerPC::PowerPCState& ppc_state) : m_ppc_state(ppc_state) {}

#define FP_OP(name) static void name(Interpreter& interpreter, UGeckoInstruction inst);
  FP_OP(faddsx) FP_OP(fdivsx) FP_OP(fmaddsx) FP_OP(fmsubsx) FP_OP(fmulsx) FP_OP(fnmaddsx)
  FP_OP(fnmsubsx) FP_OP(fresx) FP_OP(fsubsx) FP_OP(fabsx) FP_OP(fcmpo) FP_OP(fcmpu)
  FP_OP(fctiwx) FP_OP(fctiwzx) FP_OP(fmrx) FP_OP(fnabsx) FP_OP(fnegx) FP_OP(frspx)
  FP_OP(faddx) FP_OP(fdivx) FP_OP(fmaddx) FP_OP(fmsubx) FP_OP(fmulx) FP_OP(fnmaddx)
  FP_OP(fnmsubx) FP_OP(frsqrtex) FP_OP(fselx) FP_OP(fsubx)
  FP_OP(ps_div) FP_OP(ps_sub) FP_OP(ps_add) FP_OP(ps_sel) FP_OP(ps_res) FP_OP(ps_mul)
  FP_OP(ps_rsqrte) FP_OP(ps_msub) FP_OP(ps_madd) FP_OP(ps_nmsub) FP_OP(ps_nmadd)
  FP_OP(ps_neg) FP_OP(ps_mr) FP_OP(ps_nabs) FP_OP(ps_abs) FP_OP(ps_sum0) FP_OP(ps_sum1)
  FP_OP(ps_muls0) FP_OP(ps_muls1) FP_OP(ps_madds0) FP_OP(ps_madds1) FP_OP(ps_cmpu0)
  FP_OP(ps_cmpo0) FP_OP(ps_cmpu1) FP_OP(ps_cmpo1) FP_OP(ps_merge00) FP_OP(ps_merge01)
  FP_OP(ps_merge10) FP_OP(ps_merge11)
#undef FP_OP

  static void Helper_FloatCompareOrdered(PowerPC::PowerPCState& ppc_state, UGeckoInstruction inst,
                                         double a, double b);
  static void Helper_FloatCompareUnordered(PowerPC::PowerPCState& ppc_state, UGeckoInstruction inst,
                                           double a, double b);

  PowerPC::PowerPCState& m_ppc_state;
};
