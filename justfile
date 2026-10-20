# Needs the capnp tool and `cargo install capnpc --version 0.27.0`, the
# plugin for the capnp crate this workspace depends on.
regen-schema:
    capnp compile -orust:crates/claimdag/src --src-prefix=schema schema/claimdag.capnp
    rustfmt --edition 2021 crates/claimdag/src/claimdag_capnp.rs

check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --no-fail-fast

# What the graph costs as it fills toward its node cap.
bench sizes="":
    cargo run --release -p claimdag --example bench -- {{sizes}}
