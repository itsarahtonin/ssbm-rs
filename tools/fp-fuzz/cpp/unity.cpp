// Compiles Dolphin's float instructions inside their own namespace, so they link into the test
// binary next to the Ishiiruka transcription. build.rs sets the include path and SHIM_PREFIX.
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
