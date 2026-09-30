// C entry points. Included inside the Dolphin namespace.

#define SHIM_CAT2(a, b) a##b
#define SHIM_CAT(a, b) SHIM_CAT2(a, b)
#define SHIM(name) SHIM_CAT(SHIM_PREFIX, name)

static bool ExecuteFp(Interpreter& in, UGeckoInstruction inst)
{
  const u32 sub5 = inst.SUBOP5;
  const u32 sub10 = inst.SUBOP10;
  switch (inst.OPCD)
  {
  case 59:
    switch (sub5)
    {
    case 18: Interpreter::fdivsx(in, inst); return true;
    case 20: Interpreter::fsubsx(in, inst); return true;
    case 21: Interpreter::faddsx(in, inst); return true;
    case 24: Interpreter::fresx(in, inst); return true;
    case 25: Interpreter::fmulsx(in, inst); return true;
    case 28: Interpreter::fmsubsx(in, inst); return true;
    case 29: Interpreter::fmaddsx(in, inst); return true;
    case 30: Interpreter::fnmsubsx(in, inst); return true;
    case 31: Interpreter::fnmaddsx(in, inst); return true;
    }
    return false;
  case 63:
    switch (sub5)
    {
    case 18: Interpreter::fdivx(in, inst); return true;
    case 20: Interpreter::fsubx(in, inst); return true;
    case 21: Interpreter::faddx(in, inst); return true;
    case 23: Interpreter::fselx(in, inst); return true;
    case 25: Interpreter::fmulx(in, inst); return true;
    case 26: Interpreter::frsqrtex(in, inst); return true;
    case 28: Interpreter::fmsubx(in, inst); return true;
    case 29: Interpreter::fmaddx(in, inst); return true;
    case 30: Interpreter::fnmsubx(in, inst); return true;
    case 31: Interpreter::fnmaddx(in, inst); return true;
    }
    switch (sub10)
    {
    case 0: Interpreter::fcmpu(in, inst); return true;
    case 12: Interpreter::frspx(in, inst); return true;
    case 14: Interpreter::fctiwx(in, inst); return true;
    case 15: Interpreter::fctiwzx(in, inst); return true;
    case 32: Interpreter::fcmpo(in, inst); return true;
    case 40: Interpreter::fnegx(in, inst); return true;
    case 72: Interpreter::fmrx(in, inst); return true;
    case 136: Interpreter::fnabsx(in, inst); return true;
    case 264: Interpreter::fabsx(in, inst); return true;
    }
    return false;
  case 4:
    switch (sub5)
    {
    case 10: Interpreter::ps_sum0(in, inst); return true;
    case 11: Interpreter::ps_sum1(in, inst); return true;
    case 12: Interpreter::ps_muls0(in, inst); return true;
    case 13: Interpreter::ps_muls1(in, inst); return true;
    case 14: Interpreter::ps_madds0(in, inst); return true;
    case 15: Interpreter::ps_madds1(in, inst); return true;
    case 18: Interpreter::ps_div(in, inst); return true;
    case 20: Interpreter::ps_sub(in, inst); return true;
    case 21: Interpreter::ps_add(in, inst); return true;
    case 23: Interpreter::ps_sel(in, inst); return true;
    case 24: Interpreter::ps_res(in, inst); return true;
    case 25: Interpreter::ps_mul(in, inst); return true;
    case 26: Interpreter::ps_rsqrte(in, inst); return true;
    case 28: Interpreter::ps_msub(in, inst); return true;
    case 29: Interpreter::ps_madd(in, inst); return true;
    case 30: Interpreter::ps_nmsub(in, inst); return true;
    case 31: Interpreter::ps_nmadd(in, inst); return true;
    }
    switch (sub10)
    {
    case 0: Interpreter::ps_cmpu0(in, inst); return true;
    case 32: Interpreter::ps_cmpo0(in, inst); return true;
    case 40: Interpreter::ps_neg(in, inst); return true;
    case 64: Interpreter::ps_cmpu1(in, inst); return true;
    case 72: Interpreter::ps_mr(in, inst); return true;
    case 96: Interpreter::ps_cmpo1(in, inst); return true;
    case 136: Interpreter::ps_nabs(in, inst); return true;
    case 264: Interpreter::ps_abs(in, inst); return true;
    case 528: Interpreter::ps_merge00(in, inst); return true;
    case 560: Interpreter::ps_merge01(in, inst); return true;
    case 592: Interpreter::ps_merge10(in, inst); return true;
    case 624: Interpreter::ps_merge11(in, inst); return true;
    }
    return false;
  }
  return false;
}

// Runs one float instruction. `fpr` holds ps0/ps1 pairs for f0-f31; it, FPSCR and CR are
// updated in place.
// Returns 0 if the instruction is not a float instruction this shim knows.
extern "C" int SHIM(exec)(uint32_t hex, uint64_t* fpr, uint32_t* fpscr, uint32_t* cr)
{
  PowerPC::PowerPCState state{};
  for (int i = 0; i < 32; i++)
    state.ps[i].SetBoth(fpr[2 * i], fpr[2 * i + 1]);
  state.fpscr.Hex = *fpscr;
  state.cr.Set(*cr);
  Interpreter in(state);
  if (!ExecuteFp(in, UGeckoInstruction(hex)))
    return 0;
  for (int i = 0; i < 32; i++)
  {
    fpr[2 * i] = state.ps[i].PS0AsU64();
    fpr[2 * i + 1] = state.ps[i].PS1AsU64();
  }
  *fpscr = state.fpscr.Hex;
  *cr = state.cr.Get();
  return 1;
}

extern "C" uint64_t SHIM(convert_to_double)(uint32_t value)
{
  return ConvertToDouble(value);
}

extern "C" uint32_t SHIM(convert_to_single)(uint64_t value)
{
  return ConvertToSingle(value);
}

extern "C" uint32_t SHIM(convert_to_single_ftz)(uint64_t value)
{
  return ConvertToSingleFTZ(value);
}
