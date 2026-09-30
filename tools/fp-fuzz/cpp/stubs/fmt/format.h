// Just enough of fmt for BitField.h's formatter specializations to parse.
#pragma once

namespace fmt
{
struct format_parse_context
{
};
template <typename T, typename Char = char, typename Enable = void>
struct formatter
{
};
}  // namespace fmt
