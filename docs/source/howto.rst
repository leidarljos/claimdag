Point at a graph
================

``--dir DIR``, else ``CLAIMDAG_DIR``, else ``$XDG_RUNTIME_DIR/claimdag`` (no runtime directory: ``$XDG_STATE_HOME/claimdag``, else ``~/.local/state/claimdag``, else ``/tmp/claimdag-UID``). The
working directory is never used, so a worker in the wrong directory does
not invent a second graph. A reader that finds no graph says so rather than
answering "nothing claimable".

Pick the next node
==================

``claimdag list`` shows live work newest first. The ready nodes are the ones
to claim; among them, the one with the longest chain of dependents below it
frees the most work. The Model Context Protocol (MCP) tool ``claimdag_ready`` returns them in that
order with ``waiting_below`` on each row, the critical-path rule of Hu
(doi:10.1287/opre.9.6.841) and Graham (doi:10.1137/0117039).

Take work with several workers at once
======================================

.. code:: console

   $ claimdag claim-next --assignee $me --role verifier
   f7b7df44c9323a9e2a4aee09c5c74725  gen=2

Several workers can read the ready list at once and claim its head. All
but one are refused. ``claim-next`` chooses and claims under the directory
lock, so the next worker in sees the claim.

Claim from a tracker id
=======================

claimdag knows nothing about tickets. The seat maps a tracker id to one
node and a name to one actor, so ``ljos claim proj-1a2b --assignee you``
works; see the ljos site. On its own, claimdag takes 32-hex ids only.

Fence a completion
==================

.. code:: console

   $ gen=$(claimdag claim $id --assignee $me | sed 's/gen=//')
   $ ... work ...
   $ claimdag complete $id --status done --gen $gen

Pass the generation you were given. If the node was reclaimed and taken by
someone else in between, the completion is refused.

Hand back stalled claims
========================

.. code:: console

   $ claimdag reclaim --lease 900

Every claim quiet for longer than 900 seconds returns to ready. Run it from
a timer, or from the pane with one key.

Keep the list short
===================

.. code:: console

   $ claimdag archive $id
   $ claimdag list --terminal     # finished, not archived
   $ claimdag list --all          # everything

Archive sets a flag; the node and its terminal status stay.

Drive the pane
==============

.. code:: console

   $ claimdag-tui

The pane shows live work and how long each held node has been quiet. It
claims and completes through the same graph calls the command line makes,
and hands back every stale claim with one key. It relaxes none of the
guards.

Serve to an agent
=================

.. code:: console

   $ claimdag-mcp

The tools that read are ``claimdag_ready``, ``claimdag_ready_balanced`` and
``claimdag_list``. The rest change the graph: ``claimdag_claim``,
``claimdag_claim_next``, ``claimdag_renew``, ``claimdag_release``,
``claimdag_reopen``, ``claimdag_complete`` and ``claimdag_reclaim``. Two prompts
sequence taking the next node under a lease and sweeping stale claims.
