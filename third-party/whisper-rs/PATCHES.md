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

Regression checks:

```sh
cargo test --locked --test whisper_language
```

The application also avoids the upstream safe callback setters, which leak
their callback boxes. Its scoped immutable callback owner is in
src/transcribe.rs; it remains alive until the synchronous full call returns.
