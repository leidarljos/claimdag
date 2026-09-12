======
How-to
======



Point at a graph
----------------

``--dir DIR``, else ``CLAIMDAG_DIR``, else ``$XDG_RUNTIME_DIR/claimdag``. The
working directory is never used, so a worker in the wrong directory does
not invent a second graph. A reader that finds no graph says so rather than
answering "nothing claimable".

Pick the next node
------------------

``claimdag list`` shows live work newest first. The ready nodes are the ones
to claim; among them, the one with the longest chain of dependents below it
frees the most work. The Model Context Protocol (MCP) tool ``claimdag_ready`` returns them in that
order with ``waiting_below`` on each row, the critical-path rule of Hu
(https://doi.org/10.1287/opre.9.6.841) and Graham (https://doi.org/10.1137/0117039).

Claim from a tracker id
-----------------------

claimdag knows nothing about tickets. The seat maps a tracker id to one
node and a name to one actor, so ``ljos claim proj-1a2b --assignee you``
works; see the ljos site. On its own, claimdag takes 32-hex ids only.

Fence a completion
------------------

.. code:: console

    $ gen=$(claimdag claim $id --assignee $me | sed 's/gen=//')
    $ ... work ...
    $ claimdag complete $id --status done --gen $gen

Pass the generation you were given. If the node was reclaimed and taken by
someone else in between, the completion is refused.

Hand back stalled claims
------------------------

.. code:: console

    $ claimdag reclaim --lease 900

Every claim quiet for longer than 900 seconds returns to ready. Run it from
a timer, or from the pane with one key.

Keep the list short
-------------------

.. code:: console

    $ claimdag archive $id
    $ claimdag list --terminal     # finished, not archived
    $ claimdag list --all          # everything

Archive sets a flag; the node and its terminal status stay.

Drive the pane
--------------

.. code:: console

    $ claimdag-tui

The pane shows live work and how long each held node has been quiet. It
claims and completes through the same graph calls the command line makes,
and hands back every stale claim with one key. It relaxes none of the
guards.

Serve to an agent
-----------------

.. code:: console

    $ claimdag-mcp

Tools: ``claimdag_ready``, ``claimdag_list``, ``claimdag_get``, ``claimdag_claim``,
``claimdag_renew``, ``claimdag_complete``, ``claimdag_reclaim``. Two prompts
sequence taking the next node under a lease and sweeping stale claims.
