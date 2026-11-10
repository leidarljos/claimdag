.. _archive-finished-work-1:

Archive finished work
=====================

``archive`` sets a flag on a terminal node. The status stays terminal. The
node stays in the snapshot. ``list`` shows live work. ``list --terminal`` adds
finished nodes that are not archived. ``list --archived`` adds archived
nodes. ``list --all`` shows every node.

.. code:: console

   $ export CLAIMDAG_DIR=/tmp/demo-claims
   $ me=$(printf '%032x' 7)
   $ id=$(claimdag upsert --summary "land the adapter")
   $ claimdag claim $id --assignee $me
   gen=2
   $ claimdag complete $id --status done --actor $me
   092dca29...  done
   $ claimdag list --terminal
   092dca29...  done  task  00000000  gen=2  land the adapter
   $ claimdag archive $id
   092dca29...  archived
   $ claimdag list --terminal
   $ claimdag list --all
   092dca29...  done  task  00000000  gen=3  archived  land the adapter

``unarchive`` clears the flag. The status stays terminal, and the generation
moves, as ``archive`` moved it. ``archive`` on a node that is still live is
refused and names the command that finishes it. The
:doc:`reference <reference>` has both flags. The
:doc:`concepts <concepts>` page is the flag's place on the node. The
:doc:`pane <howto-pane>` toggles the same views with ``h`` and ``A``.
