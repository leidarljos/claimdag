.. _take-the-next-node-1:

Take the next node
==================

``claimdag list`` shows live work, newest first. ``claimdag ready`` shows the
nodes that can be taken: unblocked and unfinished, deepest chain first.
Each row carries ``below=N``, the unfinished chain under that node. The rule
is the critical path of Hu (doi:10.1287/opre.9.6.841) and Graham
(doi:10.1137/0117039). Take the first row.

.. code:: console

   $ export CLAIMDAG_DIR=/tmp/demo-claims
   $ a=$(claimdag upsert --summary "parse the manifest header")
   $ b=$(claimdag upsert --summary "reject a manifest with no header" --parent $a)
   $ claimdag link $a $b
   $ claimdag ready
   092dca29...  ready  task  gen=1  below=1  parse the manifest header
   $ me=$(printf '%032x' 7)
   $ claimdag ready --balanced --assignee $me --slack 1
   092dca29...  ready  task  gen=1  below=1  parse the manifest header
   $ claimdag claim-next --assignee $me
   092dca29...  gen=2
   $ claimdag ready

``ready --balanced`` orders by role affinity and by a hash of the assignee,
among nodes within ``--slack`` of the deepest ready chain (default 1).
``claim-next``
claims the node that order picks, under the directory lock, and prints the
id and the new generation. The next worker in sees the claim. Several
workers that all ``claim`` the same id are refused after the first. The
:doc:`explanation <explanation>` cites the scheduling papers, including
HEFT for workers of different speeds.

``--role`` on ``ready`` and on ``claim-next`` prefers a node whose role matches.
The roles are on the :doc:`concepts <concepts>` page. The
:doc:`lease how-to <howto-lease>` is what to do with the generation
``claim-next`` printed. The server tool for the same choice is
``claimdag_claim_next``, on the :doc:`server <howto-mcp>` page.
