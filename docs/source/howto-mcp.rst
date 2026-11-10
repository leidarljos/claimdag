.. _serve-the-graph-1:

Serve the graph
===============

``claimdag-mcp`` speaks the Model Context Protocol on standard input and
output. It opens the directory from :doc:`Point at a graph <howto-dir>`.
Install it from the :doc:`installation <install>` page.

The tools that read are ``claimdag_ready``, ``claimdag_ready_balanced``,
``claimdag_list``, and ``claimdag_get``. ``claimdag_ready`` returns ready nodes
deepest chain first, with ``waiting_below`` on each row. ``claimdag_ready_balanced``
orders by role and by worker, the same rule as ``ready --balanced`` in
:doc:`Take the next node <howto-take>`.

The tools that write are ``claimdag_upsert``, ``claimdag_link``,
``claimdag_unlink``, ``claimdag_claim``, ``claimdag_claim_next``,
``claimdag_renew``, ``claimdag_release``, ``claimdag_reopen``,
``claimdag_complete``, and ``claimdag_reclaim``. A writing tool takes the
directory lock from load to save. When the call names no actor, or names
a different actor, the tool result carries a text block after
the row: ``allow``, ``warn``, and ``no-actor`` or ``not-assignee``, separated by
tabs. That is the same line stderr and ``warn.log`` get.

Two prompts sequence a sitting:

====================== ================================================================================================================================================================
Prompt                 What it asks the caller to do
====================== ================================================================================================================================================================
``take_the_next_node`` ``claimdag_ready``, then ``claimdag_claim``, carry the generation, ``claimdag_renew`` while the work is in hand, then ``claimdag_complete`` with that generation
``sweep_stale_claims`` ``claimdag_reclaim`` for claims quiet longer than the lease, default 900 seconds
====================== ================================================================================================================================================================

The prompt arguments are an optional actor and an optional lease in
seconds. The :doc:`lease how-to <howto-lease>` is the same sequence on
the command line. The :doc:`reference <reference>` is the verb each
tool wraps.
