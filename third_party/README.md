# Vendored crates

`xattr` 0.2.3 is kept here, with one change, because it stops compiling on Linux against current `libc`: it refers to
`libc::ENOATTR`, which newer `libc` releases no longer define on Linux. On Linux `ENOATTR` is the same error as
`ENODATA`, so `src/sys/mod.rs` uses that. The crate comes in through `gpui_http_client` and `zed-async-tar`. Its licences
are kept beside the source. Remove this patch when the upstream dependency is fixed.
