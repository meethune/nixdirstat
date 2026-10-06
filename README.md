# NixDirStat

Disk usage analyzer and cleanup assistant for Unix and Unix-like systems.

Scan local filesystems, devices, and directories then explore disk usage through sortable file lists, file-type statistics, and interactive treemaps.

## Platforms

- Linux (ext4, xfs, btrfs)
- macOS (APFS)
- FreeBSD (ZFS, UFS)

## Installation

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

## Usage

```bash
# Scan a directory and open the interactive TUI
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

Requires Rust 1.95+ (edition 2024).

```bash
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
test
