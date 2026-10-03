// File lookups: only the resampling coefficients, which the shim supplies (ax_dolphin_coefs).
#pragma once

#include <string>
#include <vector>

#include "Common/CommonTypes.h"

namespace AxDolphin
{
inline std::vector<u8>& Coefs()
{
  static std::vector<u8> coefs;
  return coefs;
}
}  // namespace AxDolphin

enum
{
  D_GCUSER_IDX
};

namespace File
{
inline std::string GetUserPath(int)
{
  return "user/";
}
inline std::string GetSysDirectory()
{
  return "sys";
}
inline u64 GetSize(const std::string& name)
{
  return name == "sys/GC/dsp_coef.bin" ? AxDolphin::Coefs().size() : 0;
}
}  // namespace File
