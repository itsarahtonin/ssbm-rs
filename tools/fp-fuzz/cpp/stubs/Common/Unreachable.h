#pragma once

#include <cstdlib>

namespace Common
{
[[noreturn]] inline void Unreachable()
{
  std::abort();
}
}  // namespace Common
