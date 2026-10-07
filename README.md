# NixDirStat

Disk usage analyzer and cleanup assistant for Unix and Unix-like systems.

Scan local filesystems, devices, and directories then explore disk usage through sortable file lists, file-type statistics, and interactive treemaps.

## Platforms

- Linux (ext4, xfs, btrfs)
- macOS (APFS)
- FreeBSD (ZFS, UFS)

## Installation

### Homebrew (macOS and Linux)

```bash
brew install meethune/nixdirstat/nixdirstat
```

### From Source

```bash
cargo install nixdirstat
```

### From Release Binaries

Download the latest release from [GitHub Releases](https://github.com/meethune/nixdirstat/releases).

Binaries are available for:
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `universal-apple-darwin` (fat binary for Intel + Apple Silicon)
- `x86_64-unknown-freebsd`

Each archive includes shell completions for bash, zsh, and fish.

## Features

- **Interactive TUI** — recursive treemap, directory tree, per-extension color legend
- **Fast scanning** — parallel filesystem walk with hardlink dedup
- **SQLite storage** — scan once, explore later; export to CSV or JSON
- **Cross-platform** — portable POSIX design, tested on Linux, macOS, FreeBSD

## Usage

```bash
# Scan a directory and open the interactive TUI
nixdirstat /path/to/directory

# Equivalent explicit form
nixdirstat scan /path/to/directory

# Scan and save results to a file (batch mode)
nixdirstat scan /path/to/directory --output scan.db

# Explore a previously saved scan
nixdirstat explore scan.db

# Export scan results
nixdirstat export scan.db --format json
nixdirstat export scan.db --format csv
```

## Development

Requires Rust 1.95+ (edition 2024) and [`just`](https://github.com/casey/just).

```bash
# First-time setup — installs all tools and verifies everything works
just setup

# Full CI check (format, lint, test, doc, deny)
just check

# Build and run
cargo run -- scan /tmp

# Run tests
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development conventions, cross-platform guidelines, and the full contributor workflow.

## License

MIT
