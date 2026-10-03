// Save states, which the reference doesn't need.
#pragma once

class PointerWrap
{
public:
  template <typename T>
  void Do(T&)
  {
  }
  bool IsReadMode() const { return false; }
  void SetVerifyMode() {}
};
