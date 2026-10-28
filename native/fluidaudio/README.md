# Local Nemotron runtime subset

Selected files from [FluidAudio](https://github.com/FluidInference/FluidAudio/tree/1b0ed50d686873128a4129b3510ce3914c849bab), commit `1b0ed50d686873128a4129b3510ce3914c849bab`, under Apache-2.0 (LICENSE alongside these files).

Only Nemotron 3 streaming configuration/state/inference, mel features and their memory helpers are included. Application behavior and persistence remain in Rust. The download/cache mutation API and unrelated memory extensions were removed. Logging is a small stderr-only shim. Float16 readback uses exact element conversion because the installed Swift 5.8/macOS 13.3 SDK lacks the newer Accelerate overload. No model algorithms, thresholds or weights are changed by that compatibility adaptation.

`../model-runtime.swift` is the local, JSON-file-driven executable wrapper. It accepts already installed model paths and decoded 16kHz float32 PCM; it contains no networking. Rust owns deliberate model installation, checksums, admission, job provenance, common source offsets, and the model lease. Model weights are separate downloads with their own upstream terms. Vendoring this subset avoids requiring a newer Swift toolchain merely to build the complete SDK package.

## Parakeet TDT ASR subset

The ASR files (`Asr*.swift`, `Tdt*.swift`, `ChunkProcessor`, `ANEOptimizer`, `MLArrayCache`, `ProgressEmitter`, `AudioConverter` and small helpers) come from [FluidAudio v0.9.1](https://github.com/FluidInference/FluidAudio/tree/915a392a3b214205fcf4860cc3608bde35891d03), Apache-2.0. They are newer than the Nemotron files above. Local changes: `AsrModels.swift` loads an installed bundle from a directory and has no download code; `ModelNames.swift` keeps only the ASR file names; `PerformanceMetrics.swift` drops the memory monitor (it needs a C helper); `Support.swift` gained `error`. `../asr.swift` is the request wrapper. Compiled in Swift 5 language mode because Swift 6.3 rejects some of the upstream concurrency code.
