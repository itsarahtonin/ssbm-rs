// Dolphin's UCodeInterface (UCodes.h), without microcode switching, which the reference never
// does.
#pragma once

#include <cstring>
#include <memory>

#include "Common/CommonTypes.h"
#include "Core/HW/DSPHLE/DSPHLE.h"

class PointerWrap;

namespace DSP::HLE
{
#define UCODE_ROM 0x00000000
#define UCODE_INIT_AUDIO_SYSTEM 0x00000001
#define UCODE_NULL 0xFFFFFFFF

class UCodeInterface
{
public:
  UCodeInterface(DSPHLE* dsphle, u32 crc)
      : m_mail_handler(dsphle->AccessMailHandler()), m_dsphle(dsphle), m_crc(crc)
  {
  }
  virtual ~UCodeInterface() = default;
  virtual void Initialize() = 0;
  virtual void HandleMail(u32 mail) = 0;
  virtual void Update() = 0;
  virtual void DoState(PointerWrap& p) = 0;

protected:
  void PrepareBootUCode(u32) {}
  bool NeedsResumeMail() { return false; }
  void DoStateShared(PointerWrap&) {}

  CMailHandler& m_mail_handler;

  static constexpr u32 TASK_MAIL_MASK = 0xFFFF'0000;
  static constexpr u32 TASK_MAIL_TO_CPU = 0xDCD1'0000;
  static constexpr u32 DSP_INIT = TASK_MAIL_TO_CPU | 0x0000;
  static constexpr u32 DSP_RESUME = TASK_MAIL_TO_CPU | 0x0001;
  static constexpr u32 DSP_YIELD = TASK_MAIL_TO_CPU | 0x0002;
  static constexpr u32 DSP_DONE = TASK_MAIL_TO_CPU | 0x0003;
  static constexpr u32 DSP_SYNC = TASK_MAIL_TO_CPU | 0x0004;
  static constexpr u32 DSP_FRAME_END = TASK_MAIL_TO_CPU | 0x0005;
  static constexpr u32 TASK_MAIL_TO_DSP = 0xCDD1'0000;
  static constexpr u32 MAIL_RESUME = TASK_MAIL_TO_DSP | 0x0000;
  static constexpr u32 MAIL_NEW_UCODE = TASK_MAIL_TO_DSP | 0x0001;
  static constexpr u32 MAIL_RESET = TASK_MAIL_TO_DSP | 0x0002;
  static constexpr u32 MAIL_CONTINUE = TASK_MAIL_TO_DSP | 0x0003;

  bool m_upload_setup_in_progress = false;
  DSPHLE* m_dsphle;
  u32 m_crc;
};
}  // namespace DSP::HLE
