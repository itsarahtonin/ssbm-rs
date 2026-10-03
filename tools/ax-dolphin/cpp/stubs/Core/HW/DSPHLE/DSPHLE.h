// The DSP's HLE: the system and mail it gives microcodes.
#pragma once

#include "Common/CommonTypes.h"
#include "Core/HW/DSPHLE/MailHandler.h"
#include "Core/System.h"

namespace DSP::HLE
{
class DSPHLE
{
public:
  Core::System& GetSystem() { return m_system; }
  CMailHandler& AccessMailHandler() { return m_mail; }
  void SetUCode(u32) {}
  void SwapUCode(u32) {}

private:
  Core::System m_system;
  CMailHandler m_mail;
};
}  // namespace DSP::HLE
