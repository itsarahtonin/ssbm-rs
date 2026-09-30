// SPDX-License-Identifier: GPL-3.0-or-later

//! C `printf` formatting over EABI variadic arguments, for `OSReport` and `OSPanic`.

use ssbm_rt::Ctx;

/// Variadic arguments as the callee sees them: GPRs from `r(first)`, FPRs from f1 (only when
/// the caller set CR bit 6), then the caller's stack words at 8(r1).
pub(crate) struct VaArgs<'a> {
    ctx: &'a Ctx,
    gpr: usize,
    fpr: usize,
    stack: u32,
}

impl<'a> VaArgs<'a> {
    pub fn new(ctx: &'a Ctx, first_gpr: usize) -> Self {
        let fp_in_regs = ctx.regs.cr.get() & (1 << (31 - 6)) != 0;
        Self {
            ctx,
            gpr: first_gpr,
            fpr: if fp_in_regs { 1 } else { 9 },
            stack: ctx.regs.r(1) + 8,
        }
    }

    fn stack_word(&mut self) -> u32 {
        let v = self.ctx.read_u32(self.stack);
        self.stack += 4;
        v
    }

    pub fn u32(&mut self) -> u32 {
        if self.gpr <= 10 {
            self.gpr += 1;
            self.ctx.regs.r(self.gpr - 1)
        } else {
            self.stack_word()
        }
    }

    pub fn u64(&mut self) -> u64 {
        if self.gpr.is_multiple_of(2) {
            self.gpr += 1;
        }
        if self.gpr < 10 {
            self.gpr += 2;
            (u64::from(self.ctx.regs.r(self.gpr - 2)) << 32)
                | u64::from(self.ctx.regs.r(self.gpr - 1))
        } else {
            self.gpr = 11;
            self.stack = (self.stack + 7) & !7;
            (u64::from(self.stack_word()) << 32) | u64::from(self.stack_word())
        }
    }

    pub fn f64(&mut self) -> f64 {
        if self.fpr <= 8 {
            self.fpr += 1;
            self.ctx.regs.f(self.fpr - 1)
        } else {
            self.stack = (self.stack + 7) & !7;
            f64::from_bits(self.u64_stack())
        }
    }

    fn u64_stack(&mut self) -> u64 {
        (u64::from(self.stack_word()) << 32) | u64::from(self.stack_word())
    }
}

pub(crate) fn cstr(ctx: &Ctx, addr: u32) -> String {
    let mut bytes = Vec::new();
    let mut a = addr;
    while bytes.len() < 4096 {
        let b = ctx.read_u8(a);
        if b == 0 {
            break;
        }
        bytes.push(b);
        a += 1;
    }
    // The game's strings are Shift-JIS or ASCII; show non-ASCII bytes as escapes.
    bytes
        .iter()
        .map(|&b| {
            if b.is_ascii() {
                char::from(b).to_string()
            } else {
                format!("\\x{b:02X}")
            }
        })
        .collect()
}

/// Formats the C format string at `fmt` with `args`.
pub(crate) fn format(ctx: &Ctx, fmt: u32, args: &mut VaArgs) -> String {
    let spec = cstr(ctx, fmt);
    let mut out = String::new();
    let mut chars = spec.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut flags = String::new();
        while let Some(&f) = chars.peek() {
            if "-+ #0".contains(f) {
                flags.push(f);
                chars.next();
            } else {
                break;
            }
        }
        let mut width = None;
        if chars.peek() == Some(&'*') {
            chars.next();
            width = Some(args.u32() as i32 as usize);
        } else {
            let mut w = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                w.push(d);
                chars.next();
            }
            width = w.parse().ok().or(width);
        }
        let mut precision = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            if chars.peek() == Some(&'*') {
                chars.next();
                precision = Some(args.u32() as usize);
            } else {
                let mut p = String::new();
                while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                    p.push(d);
                    chars.next();
                }
                precision = Some(p.parse().unwrap_or(0));
            }
        }
        let mut long_long = false;
        while let Some(&l) = chars.peek() {
            match l {
                'l' => {
                    chars.next();
                    if chars.peek() == Some(&'l') {
                        chars.next();
                        long_long = true;
                    }
                }
                'h' | 'L' | 'z' | 't' | 'j' => {
                    chars.next();
                }
                _ => break,
            }
        }
        let Some(conv) = chars.next() else { break };
        let left = flags.contains('-');
        let zero = flags.contains('0') && !left;
        let body = match conv {
            '%' => "%".to_owned(),
            'd' | 'i' => {
                let v = if long_long {
                    args.u64() as i64
                } else {
                    i64::from(args.u32() as i32)
                };
                let mut s = v.unsigned_abs().to_string();
                if let Some(p) = precision {
                    while s.len() < p {
                        s.insert(0, '0');
                    }
                }
                if v < 0 {
                    s.insert(0, '-');
                } else if flags.contains('+') {
                    s.insert(0, '+');
                } else if flags.contains(' ') {
                    s.insert(0, ' ');
                }
                s
            }
            'u' | 'x' | 'X' | 'o' => {
                let v = if long_long {
                    args.u64()
                } else {
                    u64::from(args.u32())
                };
                let mut s = match conv {
                    'u' => v.to_string(),
                    'x' => format!("{v:x}"),
                    'X' => format!("{v:X}"),
                    _ => format!("{v:o}"),
                };
                if let Some(p) = precision {
                    while s.len() < p {
                        s.insert(0, '0');
                    }
                }
                if flags.contains('#') && v != 0 {
                    s.insert_str(
                        0,
                        match conv {
                            'x' => "0x",
                            'X' => "0X",
                            'o' => "0",
                            _ => "",
                        },
                    );
                }
                s
            }
            'c' => char::from(args.u32() as u8).to_string(),
            's' => {
                let p = args.u32();
                let s = if p == 0 {
                    "(null)".to_owned()
                } else {
                    cstr(ctx, p)
                };
                match precision {
                    Some(n) => s.chars().take(n).collect(),
                    None => s,
                }
            }
            'p' => format!("{:08x}", args.u32()),
            'f' | 'F' => format!("{:.*}", precision.unwrap_or(6), args.f64()),
            'e' | 'E' => {
                let s = format!("{:.*e}", precision.unwrap_or(6), args.f64());
                if conv == 'E' { s.to_uppercase() } else { s }
            }
            'g' | 'G' => format!("{}", args.f64()),
            'n' => {
                args.u32();
                String::new()
            }
            other => format!("%{other}"),
        };
        let pad = width.unwrap_or(0).saturating_sub(body.chars().count());
        if left {
            out.push_str(&body);
            out.extend(std::iter::repeat_n(' ', pad));
        } else if zero && "dixXuofFeE".contains(conv) {
            let (sign, digits) = if body.starts_with(['-', '+', ' ']) {
                body.split_at(1)
            } else {
                ("", body.as_str())
            };
            out.push_str(sign);
            out.extend(std::iter::repeat_n('0', pad));
            out.push_str(digits);
        } else {
            out.extend(std::iter::repeat_n(' ', pad));
            out.push_str(&body);
        }
    }
    out
}
