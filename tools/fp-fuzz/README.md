# fp-fuzz

Differential fuzzing of `gekko-fp` and the interpreter against Dolphin's float instructions, compiled from Dolphin's own source. This checks the floating-point behavior needed for instruction-level equivalence.

Every float instruction the game uses runs through both implementations on random inputs, constructed rounding ties, and every combination of special values. Register bits and CR must match:

- `FpMode::Hardware` against current Dolphin master's interpreter, which matches the console. Everything must match exactly.
- `FpMode::Slippi` against `cpp/ishiiruka.cpp`, a transcription of the x86-64 code that Slippi's netplay Dolphin (the Ishiiruka fork) JIT emits for each instruction, with FMA3. That Dolphin recorded the replays. A NaN matches any NaN, and the high lane of scalar double arithmetic and of `fneg`/`fabs`/`fnabs` is not compared, because in the JIT both depend on register allocation.

The load and store conversions (`lfs`, `stfs`, `psq_st`) are checked the same way.

```sh
cargo test -p fp-fuzz --release
FP_FUZZ_ITERS=2000000 cargo test -p fp-fuzz --release
```

## Vendored Dolphin source

`dolphin/` holds files from Dolphin (GPL-2.0-or-later):

- `master/` and `common/`: unmodified files from [dolphin-emu/dolphin](https://github.com/dolphin-emu/dolphin) at `5102a0339c2177575378107b76541e47cc52122d`.
- `ishiiruka/`: the `frsqrte` and `fres` estimate tables and functions, excerpted from `Source/Core/Common/MathUtil.cpp` of [project-slippi/Ishiiruka](https://github.com/project-slippi/Ishiiruka) at `60f7b63496fb6ec7b9180a04f16f3edc0ad89fe2`.

`cpp/stubs/` replaces the parts of Dolphin these files include but do not need, such as the rest of the CPU state. `cpp/unity.cpp` compiles master's float instructions inside their own namespace. `cpp/ishiiruka.cpp` follows Ishiiruka's `Jit_FloatingPoint.cpp`, `Jit_Paired.cpp` and `Jit_Util.cpp` at the same commit, and is built with AVX2 and FMA enabled.
