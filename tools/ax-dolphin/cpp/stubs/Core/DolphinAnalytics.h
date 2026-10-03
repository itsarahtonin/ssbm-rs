// Analytics, which the reference doesn't report.
#pragma once

enum class GameQuirk
{
  UsesUnimplementedAXCommand,
  UsesAXInitialTimeDelay,
  UsesAXWiimoteBiquad,
  UsesAXWiimoteLowPass,
};

class DolphinAnalytics
{
public:
  static DolphinAnalytics& Instance()
  {
    static DolphinAnalytics instance;
    return instance;
  }
  void ReportGameQuirk(GameQuirk) {}
};
