# ContextPack command reference

This document covers every command exposed by ContextPack V1.

## Executable

The packaged executable is named `contextpack.exe`. From the repository root, the release build can be run as:

```powershell
.\target\x86_64-pc-windows-msvc\release\contextpack.exe <COMMAND>
```

If the executable is on `PATH`, use `contextpack` instead.

## General commands

### Show general help

```powershell
contextpack --help
contextpack help
```

### Show the installed version

```powershell
contextpack --version
contextpack -V
```

### Show help for a command

```powershell
contextpack help validate
contextpack help collect
contextpack validate --help
contextpack collect --help
```

## `validate`

Parse a CollectionPlan, reject unknown or invalid fields, apply defaults, canonicalize set-like values and paths, and print the normalized plan as JSON.

```text
contextpack validate <PLAN>
```

Example:

```powershell
contextpack validate .\context-plan.yaml
```

Save normalized plan JSON with normal shell redirection:

```powershell
contextpack validate .\context-plan.yaml > .\normalized-plan.json
```

Validation completes before evidence collection. An invalid plan does not produce a Context Pack.

## `collect`

Validate a CollectionPlan, collect all requested evidence, calculate deterministic IDs, and render the V1 Markdown Context Pack.

```text
contextpack collect [OPTIONS] <PLAN>
```

### Write Markdown to standard output

```powershell
contextpack collect .\context-plan.yaml
```

### Write Markdown to a file

```powershell
contextpack collect .\context-plan.yaml --output .\context-pack.md
contextpack collect .\context-plan.yaml -o .\context-pack.md
```

### Write Markdown and result JSON

```powershell
contextpack collect .\context-plan.yaml `
  --output .\context-pack.md `
  --json .\context-pack.json
```

`--output` and `-o` are equivalent. If `--output` is omitted, Markdown is written to standard output. `--json` is optional and writes the typed internal `CollectionResult` representation.

Runtime failures for individual queries are recorded in the generated pack and do not stop other valid queries.

## CollectionPlan query commands

The `collect` array supports exactly five query types.

### `file`

Collect one exact UTF-8 text file.

```yaml
- id: source-file
  type: file
  path: src/main.cpp
```

### `range`

Collect an inclusive, one-based source-line range.

```yaml
- id: focused-lines
  type: range
  path: src/main.cpp
  start_line: 10
  end_line: 80
```

### `search`

Run an in-process line-oriented literal or regular-expression search.

```yaml
- id: find-calls
  type: search
  query: 'process\s*\('
  mode: regex
  paths: [src]
  extensions: [.cpp, .h, .hpp]
  case_sensitive: true
  context:
    before: 3
    after: 5
```

Available search modes:

- `literal`
- `regex`

### `symbol`

Resolve C++ declarations or definitions with Tree-sitter C++ and optionally fall back to textual evidence.

```yaml
- id: process-definition
  type: symbol
  name: App::Processor::process
  language: cpp
  roles: [definition]
  kinds: [method]
  paths: [src]
  fallback: textual
```

Available languages:

- `auto`
- `cpp`

Available roles:

- `definition`
- `declaration`

Available symbol kinds:

- `function`
- `method`
- `constructor`
- `destructor`
- `operator`
- `class`
- `struct`
- `enum`
- `namespace`

Available fallback modes:

- `textual`
- `none`

### `git`

Run one fixed read-only Git operation through direct process invocation. ContextPack never passes user input through a shell.

```yaml
- id: repository-status
  type: git
  operation: status
```

Available Git operations:

- `status`
- `branch`
- `log`
- `diff`
- `diff_stat`
- `show`
- `blame`
- `merge_base`
- `changed_files`
- `diff_check`

Diff-family operations—`diff`, `diff_stat`, `changed_files`, and `diff_check`—support these targets:

- `working_tree`: index to working tree
- `staged`: `HEAD` to index
- `baseline`: named `base` revision to the complete working tree
- `revisions`: direct `base` to `head` comparison

Examples:

```yaml
- id: working-diff
  type: git
  operation: diff
  target: working_tree

- id: staged-files
  type: git
  operation: changed_files
  target: staged

- id: baseline-stat
  type: git
  operation: diff_stat
  target: baseline
  base: HEAD~1

- id: revision-diff
  type: git
  operation: diff
  target: revisions
  base: v1.0.0
  head: HEAD

- id: recent-history
  type: git
  operation: log
  revision: HEAD
  max_entries: 10

- id: show-commit
  type: git
  operation: show
  revision: HEAD
  paths: [src]

- id: blame-lines
  type: git
  operation: blame
  path: src/main.cpp
  revision: HEAD
  start_line: 10
  end_line: 30

- id: branch-point
  type: git
  operation: merge_base
  left: HEAD
  right: origin/main
```

## Exit codes

| Code | Meaning |
|---:|---|
| `0` | Command completed successfully. Individual runtime query failures may still be represented in a generated pack. |
| `1` | CLI I/O or internal serialization failure. |
| `2` | CollectionPlan parsing or validation failure. No pack was generated. |

## Complete starter plan

The repository includes a ready-to-edit plan at [`examples/context-plan.yaml`](examples/context-plan.yaml).
