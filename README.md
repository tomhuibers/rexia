# rexia

A native Linux client for [Rex](https://www.superlogical.com/rex), built on
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) and themed by
[Noctalia](https://noctalia.dev). It attaches to Rex servers (API revisions 8–18,
including Rex v0.1.0-preview.21) and follows Noctalia's palette, corner
radius, borders, and fonts.

rexia is unofficial and isn't affiliated with Superlogical.

See [docs/parity.md](docs/parity.md) for how it compares with Rex for Mac.

## Requirements

- Nix with flakes, and direnv with nix-direnv for automatic shell loading.
- A Wayland or X11 session with Vulkan.
- A running Rex server. rexia never starts or stops one.

## Development

```sh
direnv allow          # once per checkout; loads the dev shell from flake.nix
nix run .#dev         # run from source
```

| Command | Purpose |
| --- | --- |
| `nix run .` | Build and run the packaged client |
| `nix build` | Build the package into `./result` |
| `nix run .#dev -- ARGS` | Run from source |
| `nix run .#build` | Release build with Cargo |
| `nix run .#test -- FILTER` | Run the test suite |
| `nix run .#lint` | Clippy with warnings denied |
| `nix run .#fmt` / `.#fmt-check` | Format or check Rust sources |
| `nix run .#check` | Formatting, lint, and tests |
| `nix fmt` | Format Nix files |
| `nix flake check` | Package build with tests, plus format checks |

Development commands run from the checkout root inside the dev shell. The shell
pins Zig 0.17 and the libghostty-vt source, so builds don't download anything.

### Cached Nix builds

The packaged client uses [Crane](https://crane.dev/) to compile dependencies in a
separate `buildDepsOnly` derivation and reuse its `cargoArtifacts` for Rexia.
The first build populates that cache; subsequent application-source edits rebuild
Rexia and run its tests without rebuilding the entire dependency graph. This also
applies when a NixOS configuration updates its pinned Rexia flake input.

The shared build environment retains the pinned Ghostty source, offline Zig
packages, and native libraries. Patched crates under `vendor/` are compiled from
real sources in the dependency build, not dummy stubs. A packaging-only patch
makes `ghostty-vt-sys` watch its Git submodule only when that path exists: the
registry tarball has no submodule, and watching a missing file would force Zig
rebuilds on every Cargo invocation. Changes to those crates,
Cargo dependencies, toolchain, or native build inputs invalidate the cache.
Desktop integration, runtime library wrapping and the privileged-port helper are
installed only by the final package build.

This is dependency reuse, not persistent incremental compilation of Rexia itself.
For the fastest local edit/run loop, use `nix run .#dev`. Keep the dependency
artifacts in the Nix store (or a binary cache); garbage collection can remove
unrooted artifacts and require rebuilding them.

Run `bash tests/nix-cache.sh` to verify that an application-only edit preserves
the dependency derivation while an edit to a patched dependency invalidates it.
The check uses a temporary source copy and does not compile anything.

### Privileged local ports

If Linux denies a forward's local bind, Rexia offers a free port. To also
authorize the requested port, use a system installation of the bind helper and
polkit policy, with a polkit authentication agent running in your desktop session.
The Nix package includes both; install it in the NixOS system configuration
(`environment.systemPackages`, with `security.polkit.enable = true`) so polkit
can see its policy. A per-user package or `nix run` alone doesn't register it.

For other Linux packages, install `rexia-bind-port` to
`/usr/libexec/rexia-bind-port` and `resources/dev.hackr.rexia.bind-port.policy`
to `/usr/share/polkit-1/actions/`. Both must be system-owned and not writable
by ordinary users. The helper must not be setuid; polkit authorizes each bind.
