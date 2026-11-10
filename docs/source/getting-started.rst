Install ``claimdag`` from the :doc:`installation <install>` page if it is
not already on ``PATH``. Everything below runs in a scratch directory. By the end you will have two
nodes, one waiting on the other, and a claim a second worker cannot steal.
You will see a stalled claim handed back by its lease, and a finished node
unblock the next.

1. Make two nodes, one waiting on the other
===========================================

.. code:: console

   $ export CLAIMDAG_DIR=/tmp/demo-claims
   $ a=$(claimdag upsert --summary "parse the manifest header")
   $ b=$(claimdag upsert --summary "reject a manifest with no header" --parent $a)
   $ claimdag link $a $b
   $ claimdag list
   71368334...  todo  task  gen=1  reject a manifest with no header
   092dca29...  ready  task  gen=1  parse the manifest header

Ids are 32 hex characters, minted from the node's own fields. ``link`` adds a
hard dependency: the child becomes ready when the parent is done.
Readiness is derived, never set by hand. ``list`` prints newest first, and
breaks a tie on the id, so those two lines can swap.

2. Claim, and fail to claim twice
=================================

Actors are ids too. A stable identity is any 32-hex string you keep.

.. code:: console

   $ me=$(printf '%032x' 7); you=$(printf '%032x' 8)
   $ claimdag claim $a --assignee $me
   gen=2
   $ claimdag claim $a --assignee $you
   claim: status claimed. node 092dca29... is held by 00000000000000000000000000000007 since unix .... claimdag release 092dca29... --actor 00000000000000000000000000000007

The generation moved from 1 to 2 on the claim. That number is the token
everything after carries.

3. Say you are still working, or lose the node
==============================================

.. code:: console

   $ claimdag renew $a --actor $me
   $ claimdag reclaim --lease 60

``reclaim`` hands back every claim quiet for longer than the lease and moves
its generation. A worker that stalled and wakes up later carries a stale
token, and ``complete --gen 2`` is refused instead of finishing work that is
now somebody else's.

4. Finish, and see what opened
==============================

.. code:: console

   $ claimdag complete $a --status done --actor $me --gen 2
   092dca29...  done
   $ claimdag list
   71368334...  ready  task  gen=1  reject a manifest with no header

The child became ready the moment its parent was done. Nothing here
touched a tracker: the ticket this work belongs to is still open until
somebody closes it there.

Where next
==========

The :doc:`how-to guides <howto>` cover the directory, ``claim-next``, the
lease, archive, the pane, and the server. The :doc:`concepts <concepts>`
page draws the two edges this lesson created. The
:doc:`explanation <explanation>` is why the generation fences a stale
holder. The :doc:`reference <reference>` is the command line, and the
:doc:`API reference <api>` is the library.
