.PHONY: data setup app start stop web-build check test clean

# Game tables (not in git). Build them from your own game folder: make data D2_GAME_DIR="<folder>"
GEN := crates/d2s/data/counts.rs
$(GEN): tools/gen_data.py tools/make_data.sh
	sh tools/make_data.sh "$(D2_GAME_DIR)"

data: $(GEN)

# One-time setup: Rust target for the browser version, trunk 0.22.0, Git LFS hooks.
setup:
	rustup target add wasm32-unknown-unknown
	cargo install trunk --version 0.22.0 --locked
	git lfs install

# Native app: build the release binary and run it.
app: $(GEN)
	cargo build --release --locked -p d2rs
	./target/release/d2rs

# Browser version: serve in the background on http://127.0.0.1:8080/d2rs/app/ (log: target/web.log).
start: $(GEN)
	mkdir -p target
	cd app && nohup trunk serve > ../target/web.log 2>&1 & echo $$! > target/web.pid
	@echo "started: http://127.0.0.1:8080/d2rs/app/"

stop:
	@if [ -f target/web.pid ]; then kill $$(cat target/web.pid) 2>/dev/null; rm -f target/web.pid; echo "stopped"; else echo "not running"; fi

web-build: $(GEN)
	cd app && trunk build --release

# The same checks as the public CI lint job.
check: $(GEN)
	sh tools/check_public.sh
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo clippy -p d2rs --target wasm32-unknown-unknown --locked -- -D warnings

# Private test suite: only for people with access to the private kkuzar/d2rs_test repo (checked out next to d2rs).
test:
	@if [ ! -f ../d2rs_test/run.sh ]; then echo "d2rs_test not found next to d2rs: the test suite is private"; exit 1; fi
	cd ../d2rs_test && ./run.sh

clean: stop
	cargo clean
	rm -rf app/dist
