## Summary

<!-- One or two sentences on what this PR does and why. -->

## Changes

<!-- Bulleted list of the meaningful changes. Skip if "Summary" already covers it. -->

-

## Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test --all` passes
- [ ] Updated `CHANGELOG.md` under `## [Unreleased]` if the change is user-visible
- [ ] Updated README / CONTRIBUTING / SECURITY if docs were affected
- [ ] No new `unwrap()` / `expect()` on fallible runtime paths
- [ ] No API keys, personal paths, or session memory included in the diff

## Testing

<!-- How did you verify this works? End-to-end smoke against Ollama? Unit test?
     If this is UI-affecting and you couldn't test interactively, say so. -->
