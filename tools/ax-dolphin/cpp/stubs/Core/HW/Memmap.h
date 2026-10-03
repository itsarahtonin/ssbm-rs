// Main memory, through the bus the shim is given (ax_dolphin_run): reads and writes go to the
// caller.
#pragma once

#include <cstddef>
#include <cstring>
#include <vector>

#include "Common/CommonTypes.h"
#include "Common/Swap.h"

namespace AxDolphin
{
struct Bus
{
  void* user;
  void (*read)(void* user, u32 addr, u8* out, size_t len);
  void (*write)(void* user, u32 addr, const u8* data, size_t len);
  u8 (*aram)(void* user, u32 addr);
};
inline Bus*& CurrentBus()
{
  static Bus* bus = nullptr;
  return bus;
}
}  // namespace AxDolphin

namespace Memory
{
class MemoryManager
{
public:
  template <typename T>
  void CopyFromEmuSwapped(T* data, u32 address, size_t size) const
  {
    auto* bus = AxDolphin::CurrentBus();
    bus->read(bus->user, address, reinterpret_cast<u8*>(data), size);
    for (size_t i = 0; i < size / sizeof(T); i++)
      data[i] = Swap(data[i]);
  }
  template <typename T>
  void CopyToEmuSwapped(u32 address, const T* data, size_t size)
  {
    std::vector<T> swapped(data, data + size / sizeof(T));
    for (auto& v : swapped)
      v = Swap(v);
    CopyToEmu(address, swapped.data(), size);
  }
  void CopyToEmu(u32 address, const void* data, size_t size)
  {
    auto* bus = AxDolphin::CurrentBus();
    bus->write(bus->user, address, static_cast<const u8*>(data), size);
  }
  // Only ever read through, and only one at a time.
  u8* GetPointerForRange(u32 address, size_t size)
  {
    m_range.resize(size);
    auto* bus = AxDolphin::CurrentBus();
    bus->read(bus->user, address, m_range.data(), size);
    return m_range.data();
  }

private:
  template <typename T>
  static T Swap(T v)
  {
    if constexpr (sizeof(T) == 2)
      return static_cast<T>(Common::swap16(static_cast<u16>(v)));
    else
      return static_cast<T>(Common::swap32(static_cast<u32>(v)));
  }
  std::vector<u8> m_range;
};
}  // namespace Memory
