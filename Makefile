.PHONY: build release clean check test fix install run

build:
	cargo build

# make run ARGS="status"
run:
	cargo run -q -- $(ARGS)

release:
	cargo build --release

install:
	cargo install --path .

clean:
	cargo clean

test:
	cargo test -- --quiet

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test -- --quiet
	cargo build

fix:
	cargo fmt
	cargo clippy --fix --allow-dirty --allow-staged
