# Translating NixDirStat

NixDirStat uses [rust-i18n](https://github.com/longbridge/rust-i18n) for
internationalization. All user-facing strings live in YAML catalogs under
`locales/`, one file per language. The English catalog (`locales/en.yml`)
is the authoritative baseline — every key has an English value, and
missing translations in other locales fall back to English automatically.

## Adding a new language

1. Copy `locales/en.yml` to `locales/{code}.yml` (e.g., `locales/de.yml`).

2. Translate every value. Keep the `_version: 1` header, the
   `%{variable}` placeholders, and the key names intact — only change
   the quoted strings:

   ```yaml
   _version: 1

   explorer.panel.directory-tree: " Verzeichnisbaum "
   progress.files-label: "Dateien: %{count}"
   ```

3. Build and test:

   ```bash
   cargo test                              # all tests should pass
   cargo run -- scan /tmp --lang de        # visual check
   ```

4. Open a pull request adding your `locales/{code}.yml` file.

## Key naming convention

Keys use dot-separated hierarchical names mirroring the UI structure:

| Prefix | Scope |
|--------|-------|
| `explorer.*` | Explorer view — panels, status, help, popups |
| `progress.*` | Scan progress view |
| `preview.*` | File preview errors |
| `file-type.*` | FileType labels (File, Dir, Symlink, ...) |
| `file-category.*` | FileCategory labels (Code, Image, ...) |
| `batch.*` | Batch (non-interactive) mode output |
| `error.*` | User-facing error messages |
| `ui.*` | Miscellaneous UI strings |
| `widgets.*` | Widget-specific strings |

## What not to translate

- Size units (`B`, `KiB`, `MiB`, `GiB`, `TiB`) — IEC standards
- CLI `--help` text — stays in English
- Internal error messages — developer-facing
- Keyboard shortcut labels in the help overlay — the keys themselves
  (e.g., `↑/↓ j/k`) are universal; translate only the description text

## Interpolation variables

Strings with `%{name}` placeholders substitute runtime values. The
variable names must stay exactly as they are — only translate the
surrounding text.

## Pluralization

For strings that differ between singular and plural, use separate keys:

```yaml
explorer.status.warnings-singular: " %{count} warning (w) "
explorer.status.warnings-plural: " %{count} warnings (w) "
```
