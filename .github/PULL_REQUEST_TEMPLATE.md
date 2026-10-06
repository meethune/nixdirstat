## Summary
<!-- Brief description of the changes in this PR -->

## Related Issues
<!-- Link any related issues: Fixes #123, Relates to #456 -->

## Changes
<!-- List the key changes made in this PR -->
-
-
-

## Type of Change
- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to change)
- [ ] Documentation update
- [ ] Refactoring (no functional changes)
- [ ] Performance improvement
- [ ] Test coverage improvement

## Testing
### Test Commands Run
```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo doc --no-deps
cargo deny check
```

### Cross-Platform Testing
- [ ] Tested on Linux
- [ ] Tested on macOS (or N/A)
- [ ] Tested on FreeBSD (or N/A)

### Manual Testing
<!-- Describe any manual testing performed, especially for TUI changes -->

## Checklist
### Code Quality
- [ ] My code follows the project's code style (rustfmt, clippy clean)
- [ ] No `unsafe` code is added
- [ ] No `unwrap()`, `expect()`, or `panic!()` in library code
- [ ] Cross-platform implications considered (Linux, macOS, FreeBSD)

### Testing
- [ ] I have added tests that prove my fix/feature works
- [ ] New and existing tests pass locally with `cargo test`
- [ ] Platform-specific tests use `#[cfg_attr]` appropriately

### Documentation
- [ ] I have added doc comments (`///`) for new public items
- [ ] Doc comments include examples where appropriate

### Supply Chain
- [ ] I have run `cargo deny check` and resolved any issues
- [ ] New dependencies are justified and from trusted sources
