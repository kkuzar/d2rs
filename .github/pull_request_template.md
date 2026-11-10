## Change

Describe what the change does and why.

## Checklist

- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes.
- [ ] `cargo clippy -p d2rs --target wasm32-unknown-unknown --locked -- -D warnings` passes.
- [ ] `sh tools/check_public.sh` passes.
- [ ] The change has no test code, no test data and no game files.

## Manual checks

Describe what you tested by hand (the app, the platform, the save files). The maintainers run the private test suite.
