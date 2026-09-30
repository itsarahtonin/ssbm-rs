// Compiles one Dolphin variant's float instructions inside its own namespace, so both variants
// link into one test binary. build.rs sets the include path and SHIM_PREFIX per variant.
#include "prelude.h"

namespace SHIM_NAMESPACE
{
namespace Common
{
using namespace ::Common;
}

#include "Common/FloatUtils.cpp"
#include "Core/PowerPC/ConditionRegister.cpp"
#include "Core/PowerPC/Interpreter/Interpreter_FloatingPoint.cpp"
#include "Core/PowerPC/Interpreter/Interpreter_Paired.cpp"

#include "shim.inc"
}  // namespace SHIM_NAMESPACE
