.. _fence-a-completion-1:

Fence a completion
==================

Pass the generation ``claim`` printed. ``renew`` keeps that number and moves
the timestamp. ``release`` hands one claim back, clears the assignee, and
moves the generation. ``reclaim --lease 0`` returns a claim in the same
second, which is how this page shows the fence. A production sweep uses
the default, 900 seconds. See the :doc:`explanation <explanation>` for
why the number moves.

.. code:: console

   $ export CLAIMDAG_DIR=/tmp/demo-claims
   $ me=$(printf '%032x' 7)
   $ id=$(claimdag upsert --summary "land the adapter")
   $ claimdag claim $id --assignee $me
   gen=2
   $ claimdag renew $id --actor $me
   gen=2
   $ claimdag release $id --actor $me
   gen=3
   $ claimdag claim $id --assignee $me
   gen=4
   $ claimdag reclaim --lease 0
   092dca29...  reclaimed
   $ claimdag complete $id --status done --actor $me --gen 4
   complete: gen mismatch
   $ claimdag claim $id --assignee $me
   gen=6
   $ claimdag complete $id --status done --actor $me --gen 6
   092dca29...  done

``complete --gen 4`` is refused because ``reclaim`` moved the generation. The
second claim takes the node at the new generation, and ``complete --gen 6``
finishes it. Omitting ``--gen`` skips the check. ``--actor`` names the holder.
The call still runs when the actor is absent or is someone else, and it
writes ``allow``, ``warn``, and ``no-actor`` or ``not-assignee``, tab-separated,
on stderr and in ``warn.log``. ``--force`` finishes the node for nobody and
appends a line to ``forced.log`` instead.

``reopen`` brings a terminal node back to ready and moves the generation.
The :doc:`archive how-to <howto-archive>` is the flag that hides a
finished node from the default list. The :doc:`concepts <concepts>`
page lists every call that moves the generation.
