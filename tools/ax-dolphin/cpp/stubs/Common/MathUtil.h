// MathUtil::SaturatingCast, as Dolphin's Common/MathUtil.h has it.
#pragma once

#include <concepts>
#include <limits>
#include <type_traits>
#include <utility>

namespace MathUtil
{
template <std::integral Dest, typename T>
constexpr Dest SaturatingCast(T value)
{
  constexpr Dest lo = std::numeric_limits<Dest>::min();
  constexpr Dest hi = std::numeric_limits<Dest>::max();
  if constexpr (std::is_integral_v<T>)
  {
    if (std::cmp_less(value, lo))
      return lo;
    if (std::cmp_greater(value, hi))
      return hi;
  }
  else
  {
    if (value < static_cast<T>(lo))
      return lo;
    if (value >= static_cast<T>(hi))
      return hi;
  }
  return static_cast<Dest>(value);
}
}  // namespace MathUtil
