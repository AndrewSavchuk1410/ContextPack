# ContextPack

ContextPack V1 is a deterministic, read-only repository evidence collector with C++-first language intelligence. It consumes a strict YAML `CollectionPlan` and produces a Markdown Context Pack plus optional canonical JSON.

The workspace is intentionally split into three crates:

- `contextpack-core` owns the typed plan/result model, validation, secure traversal, evidence collection, budgets, IDs, Git integration, and Tree-sitter C++ symbol engine.
- `contextpack-render` renders the fixed V1 Markdown structure.
- `contextpack-cli` exposes the `contextpack` executable.

It does not execute user-provided commands, mutate the collected repository, call an LLM, or provide plugin, MCP, GUI, or IDE functionality.

## Build and test

Rust 1.98.1 is pinned for release builds. The code declares Rust 1.88 as its minimum supported compiler.

```text
cargo build --workspace --locked
cargo test --workspace --locked
```

## CLI

Validate a plan and print its fully normalized JSON semantics:

```text
contextpack validate context-plan.yaml
```

Collect and write Markdown and canonical result JSON:

```text
contextpack collect context-plan.yaml --output context-pack.md --json context-pack.json
```

If `--output` is omitted, Markdown is written to standard output. Plan validation failures exit with code 2; I/O or internal serialization failures exit with code 1. Runtime query failures remain explicit `QueryResult`s and do not abort other queries.

See [examples/context-plan.yaml](examples/context-plan.yaml) for a complete starter plan.
See [COMMANDS.md](COMMANDS.md) for the complete CLI and CollectionPlan command reference.

## Installing from GitHub Releases

On Windows x86_64, download the latest `contextpack-v*-windows-x86_64.zip` from the repository's GitHub Releases page and extract it. The archive includes `contextpack.exe`, this README, the command reference, and an example plan. You can also download `contextpack.exe` as a standalone release asset. Run `contextpack.exe --help` from PowerShell, or add its directory to your `PATH` to invoke `contextpack` from anywhere.

## Security and determinism

- Query paths are lexical repository-relative paths and are checked again after filesystem resolution.
- Symlink targets must remain inside the canonical repository root.
- Search uses the in-process ripgrep ecosystem (`ignore`, `grep-regex`) and deliberately disables machine-specific ignore sources.
- Git is invoked directly with an argument vector, literal pathspec semantics, colors and external diffs disabled, and no shell.
- Source is accepted only as UTF-8 text, normalized to LF, and never semantically rewritten.
- SHA-256 canonical JSON projections produce `c1-`, `e1-`, and `cp1-` IDs using the first 128 bits.
- No absolute repository path, timestamp, runtime timing, terminal state, or diagnostic message text enters deterministic identity.

## Packaging

Windows is the primary packaging target:

```powershell
./scripts/package.ps1
```

This creates `dist/contextpack-v0.1.0-windows-x86_64.zip` from a locked release build. `scripts/package.sh` produces equivalent tarballs on macOS and Linux. CI builds and tests all three operating systems.
