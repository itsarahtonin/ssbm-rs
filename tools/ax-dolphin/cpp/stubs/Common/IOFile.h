// Reading the resampling coefficients the shim supplies.
#pragma once

#include <cstring>
#include <string>

#include "Common/FileUtil.h"

namespace File
{
class IOFile
{
public:
  IOFile(const std::string&, const char*) {}
  bool ReadBytes(void* data, size_t len)
  {
    const auto& c = AxDolphin::Coefs();
    if (len > c.size())
      return false;
    std::memcpy(data, c.data(), len);
    return true;
  }
};
}  // namespace File
