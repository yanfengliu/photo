//! Per-pixel colour science: white balance, tone, curves and colour operators.
//!
//! Everything here is scalar math on `[f32; 3]` values. No image buffers, no
//! threading, no I/O. See `docs/threads/current/lightroom-restart/DESIGN.md`.
