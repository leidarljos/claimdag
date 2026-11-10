``claimdag`` is the command line. ``claimdag-mcp`` is the server. ``claimdag-tui``
is the pane. Each one reads and writes the same ``work.bin``.

The :doc:`tutorial <getting-started>` assumes ``claimdag`` is on ``PATH``. The
:doc:`reference <reference>` is the command line those binaries implement.

Release archives
================

The release workflow publishes one archive per target. The file name is
``claimdag-vVERSION-TARGET.tar.gz``. Inside it, ``claimdag``, ``claimdag-mcp`` and
``claimdag-tui`` sit next to each other. ``cargo binstall`` reads the
``package.metadata.binstall`` table in each crate and downloads that archive.

============================= ====================
Target                        Host
============================= ====================
``x86_64-unknown-linux-gnu``  Linux, 64-bit x86
``aarch64-unknown-linux-gnu`` Linux, 64-bit ARM
``aarch64-apple-darwin``      macOS, Apple silicon
``x86_64-apple-darwin``       macOS, Intel
============================= ====================

Linux
=====

On x86\ :sub:`64` or aarch64 Linux, with `cargo-binstall <https://github.com/cargo-bins/cargo-binstall>`__ installed:

.. code:: console

   $ cargo binstall claimdag-cli
   $ cargo binstall claimdag-mcp
   $ cargo binstall claimdag-tui

``cargo binstall`` fetches the archive for the host target. A target with no
archive fails there. ``cargo install`` builds the same names from source.

macOS
=====

On Apple silicon the target is ``aarch64-apple-darwin``. On Intel it is
``x86_64-apple-darwin``. The same three ``cargo binstall`` lines as on Linux
select the archive from the host target.

From crates.io
==============

Any host with a Rust toolchain can build from crates.io. ``claimdag`` and
``claimdag-cli`` declare ``rust-version`` 1.85. ``claimdag-mcp`` and
``claimdag-tui`` declare ``rust-version`` 1.88.

.. code:: console

   $ cargo install --locked claimdag-cli
   $ cargo install --locked claimdag-mcp
   $ cargo install --locked claimdag-tui

``--locked`` builds the dependency set recorded in the published ``Cargo.lock``.

From a checkout
===============

The workspace ``Cargo.toml`` is the version the crates share. From the
repository root:

.. code:: console

   $ cargo build --locked --release -p claimdag-cli -p claimdag-mcp -p claimdag-tui

The binaries are ``target/release/claimdag``, ``target/release/claimdag-mcp``
and ``target/release/claimdag-tui``. The check workflow builds and tests
this workspace with rustc 1.97.1.

A library user adds the crate:

.. code:: console

   $ cargo add claimdag

The :doc:`API reference <api>` lists the items that dependency exports.
The :doc:`directory how-to <howto-dir>` is where the snapshot is created
on first write.
