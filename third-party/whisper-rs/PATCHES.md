# Local patch to whisper-rs 0.16.0

Source: the crates.io 0.16.0 package, checksum
`2088172d00f936c348d6a72f488dc2660ab3f507263a195df308a3c2383229f6`.
Its VCS revision is `7558e1b72f54f2f22a53589afb77e65681834c36`.
The upstream license is retained in LICENSE. This is the same dependency
version, with the existing whisper-rs-sys patch still in use.

`FullParams::set_language` allocated a CString with `into_raw` and never freed
it. The patch retains the CString in an Arc field: cloned parameters keep the
same stable C pointer alive, replacing a language releases the previous string,
and the final owner frees it. The native whisper_full_params ABI is unchanged.

`FullParams::set_initial_prompt` and `FullParams::set_vad_model_path` also
allocated CStrings with `into_raw`, leaking on replacement or drop. They now
retain owned, clone-safe `Arc<CString>` values and release them when replaced.
The same allocation regression test covers language, prompt, VAD path, and
a cloned grammar. Grammar storage also uses `Arc<Vec<_>>`: cloning
`FullParams` must not leave `fp.grammar_rules` pointing into the old
owner's freed allocation.
`WhisperVadContext::new` now passes a borrowed CString pointer to the
synchronous native initializer instead of leaking one on each invocation.
VAD sample lengths are now checked before conversion to C `int`, and empty
probability arrays return an empty Rust slice without constructing a slice
from a possibly null C pointer.

Regression checks:

```sh
cargo test --locked --test whisper_language
```

The application also avoids the upstream safe callback setters, which leak
their callback boxes. Its scoped immutable callback owner is in
src/transcribe.rs; it remains alive until the synchronous full call returns.
