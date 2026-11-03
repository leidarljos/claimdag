`claimdag-mcp` and `claimdag-tui` now declare Rust 1.88, the floor their dependencies build on.
Installing either on an older compiler reports that requirement from the crate itself.
CI checks each crate on the `rust-version` it declares.
