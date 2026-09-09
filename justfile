regen-schema:
    capnp compile -orust:crates/claimdag/src --src-prefix=schema schema/claimdag.capnp

check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --no-fail-fast

# What the graph costs as it fills toward its node cap.
bench sizes="":
    cargo run --release -p claimdag --example bench -- {{sizes}}
