# Contributing to d2rs

Thank you for your help. This file tells you how to build d2rs, how to check your change, and how to send it.

## Build

1. Install Rust with [rustup](https://rustup.rs).
2. If you use Linux, install `libxkbcommon-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.
3. Run `cargo build --release -p d2rs` to build the desktop program.
4. To build the web version, run `rustup target add wasm32-unknown-unknown` and `cargo install trunk --version 0.22.0 --locked`.
5. Run `cd app && trunk serve`, then open `http://127.0.0.1:8080/d2rs/app/`.

## Check your change

Run these three checks before you open a pull request. Each check must pass.

1. `cargo clippy --workspace --all-targets --locked -- -D warnings`
2. `cargo clippy -p d2rs --target wasm32-unknown-unknown --locked -- -D warnings`
3. `sh tools/check_public.sh`

CI runs the same checks. If you change `Cargo.lock`, create the file `licenses/dependencies.json` again with `cargo about`.

## Tests are private

The test suite is in a private repository, because it uses real character save files. This repository has no test code and no test data. The maintainers run the suite for each pull request. Each commit gets a `private-tests` status. Describe your manual checks in the pull request, so that the maintainers know what to test. If the suite finds a problem, a maintainer tells you the failing case.

## Pull request flow

1. Fork the repository and create a branch.
2. Make one change for one purpose.
3. Run the three checks.
4. Open a pull request and fill in the template.
5. Wait for the checks and the maintainers. The maintainers start the private suite for a pull request from a fork.
6. Answer the review comments with new commits. Do not force-push after the review starts.

## Code rules

- Write the fewest lines that solve the problem.
- Do not add `unsafe` code. The existing operating system code in `d2mem` and the console call in `app/src/main.rs` are the only exceptions.
- Do not use panics in library code. Return an error.
- Do not add test code, test data or `dev-dependencies`. The check script finds them.
- Name the game only once, in the first lines of `README.md`. Everywhere else, write "the game".
- Do not add game files (archives, executables, images, maps) to the repository.
- Do not use the em dash character.

## Other files

- Report a security problem as described in [SECURITY.md](SECURITY.md).
- Follow the [code of conduct](CODE_OF_CONDUCT.md).
