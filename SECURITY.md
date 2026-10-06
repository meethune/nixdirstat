# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |

## Reporting a Vulnerability

**Do not report security vulnerabilities through public GitHub issues.**

Instead, please use [GitHub Security Advisories](https://github.com/meethune/nixdirstat/security/advisories/new)
to report vulnerabilities privately.

You should receive an acknowledgment within 48 hours. We will provide a more
detailed response within one week, including next steps and an expected timeline
for a fix.

## Security Measures

This project employs several security measures:

- **No unsafe code**: `unsafe` is forbidden at the lint level (`unsafe_code = "forbid"`)
- **Dependency auditing**: `cargo-deny` checks licenses, advisories, and banned crates
- **Advisory scanning**: `cargo-audit` runs on every CI push and daily via cron
- **Secret scanning**: Gitleaks scans all commits for accidentally committed secrets
- **Dependency updates**: Dependabot monitors for outdated and vulnerable dependencies
- **SHA-pinned actions**: All GitHub Actions use pinned commit SHAs, not mutable tags

## Disclosure Policy

When a vulnerability is confirmed:

1. A fix is developed and tested privately
2. A new release is published with the fix
3. A security advisory is published on GitHub
4. The vulnerability is disclosed publicly after users have had time to update
