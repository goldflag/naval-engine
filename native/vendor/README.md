# Patched dependencies

Crates copied from crates.io and changed here, patched in by `native/Cargo.toml`'s `[patch.crates-io]`. They are not
workspace members (`exclude = ["vendor"]`): Clippy and the tests leave them alone. Each one's `Cargo.toml` carries an
empty `[workspace]` table, because `cargo fmt --all` reads a path dependency's manifest on its own. Without the table,
cargo's search for its workspace passes the excluding manifests and keeps going up. In a git worktree nested under
another checkout, it ends at the outer checkout's `Cargo.toml` and the native rustfmt check fails. Upgrading the crate means copying the
new release here and carrying the change over, or dropping the patch once upstream has an equivalent.

## wgpu-hal 30.0.1

Two changes in `src/dx12` (search for "Blue Water patch"). The first bounds the DX12 backend's in-memory cache of
compiled shader stages, keyed by the generated HLSL, its entry point, stage and shader model. Upstream trims it to the
last 100 compiled stages once it holds 200. A port scene registers 196 pipelines from 86 entry shaders, about 400
stages, and every scene load (a gallery ship switch, the way back to the menu, a theatre and each of its scene swaps)
makes a new renderer that asks for every one of them again. With 200 kept the stages of the last load had already pushed
out the next one's, so DXC compiled nearly all of them again each time. The patch keeps 2048 recently compiled stages
and trims past 4096.

Measured before the patch on the RTX 5070 Ti (2026-10-26, release, 3840 × 1600, the player's graphics, CPU 90–100 %
busy with other sessions' builds): port reloads in one process summed 431, 164, 327 and 158 s of pipeline build time
(Fletcher, Hood, Fletcher again, the menu fleet), the scene's first frame waiting 7–31 s for them. The second Fletcher
compiled her ship shaders again because Hood's ocean and sky stages had pushed them out.

The second: `src/dx12/disk_cache.rs` (new) and `load_shader` in `src/dx12/device.rs` keep the compiled DXIL on disk
between launches. Neither DX12 nor the driver keeps DXC's work, so every fresh process compiled every stage again: in a
theatre that was 24–34 s of a 38–50 s load (63–73 %), the ocean surface and ship shaders three quarters of it. The
application names a directory with `wgpu_hal::dx12::set_shader_disk_cache` (naval-client:
`%LOCALAPPDATA%\naval\cache\dxil`, `pipeline_cache::keep_compiled_shaders`, trimmed to 768 MiB past 1 GiB by
modification time). A stage's file is named by two 64-bit hashes of the HLSL, source name, entry point, target profile,
the device's compile flags and a format number, and carries both hashes and its payload's checksum; a file that does not
match is compiled again and rewritten. Writes go to a staging name renamed into place. Only the statically linked DXC
(`static-dxc`) is cached, since its version is the build's own; FXC and a `dxcompiler.dll` found at run time compile
every launch as upstream does. `NAVAL_DXIL_CACHE=off` turns it off for a cold measurement.