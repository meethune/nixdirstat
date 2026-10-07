# Translating NixDirStat

NixDirStat uses [rust-i18n](https://github.com/nickel-org/rust-i18n) for
internationalization. All user-facing strings live in YAML catalogs under
`locales/`. The English catalog (`locales/en.yml`) is the authoritative
baseline — every key has an English value, and missing translations in
other locales fall back to English automatically.

## Adding a new language

1. Open `locales/en.yml` and study the format. Each key maps locale codes
   to translated strings:

   ```yaml
   explorer.panel.directory-tree:
     en: " Directory Tree "
   ```

2. Add your locale code under each key:

   ```yaml
   explorer.panel.directory-tree:
     en: " Directory Tree "
     fr: " Arborescence "
   ```

3. Translate every key. Keep the `%{variable}` placeholders intact — they
   are substituted at runtime:

   ```yaml
   progress.files-label:
     en: "Files: %{count}"
     fr: "Fichiers : %{count}"
   ```

4. Build and test:

   ```bash
   cargo test                              # all tests should pass
   cargo run -- scan /tmp --lang fr        # visual check
   ```

5. Open a pull request with your changes to `locales/en.yml`.

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
explorer.status.warnings-singular:
  en: " %{count} warning (w) "
  fr: " %{count} avertissement (w) "
explorer.status.warnings-plural:
  en: " %{count} warnings (w) "
  fr: " %{count} avertissements (w) "
```
