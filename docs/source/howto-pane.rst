.. _drive-the-pane-1:

Drive the pane
==============

``claimdag-tui`` draws the parent forest. It claims, completes, and archives
through the same ``WorkGraph`` calls the command line makes. A call that
writes a warning on the command line writes it in the pane too.

.. code:: console

   $ export CLAIMDAG_DIR=/tmp/demo-claims
   $ a=$(claimdag upsert --summary "land the adapter")
   $ claimdag upsert --summary "write the tree" --parent $a
   $ claimdag-tui --dump
   ready  land the adapter
     ready  write the tree

``--dump`` prints the forest and exits. Both lines say ready: the parent
edge draws the tree, and it does not block the child. A dependency does,
which is what ``link`` records. ``--all`` with ``--dump`` includes
finished and archived work. Without ``--dump`` the program opens the
terminal and waits for a key.

======================== =======================
Key                      Action
======================== =======================
``c``                    claim the selected node
``d``                    complete it
``R``                    reclaim stale claims
``a``                    archive
``u``                    unlink
``h``                    show finished work
``A``                    show archived work
``r``                    reload
``j`` / Down, ``k`` / Up move the selection
``q`` or Esc             quit
======================== =======================

The footer in the pane draws the same bindings. Colours come from the
terminal. The lease the pane offers is 900 seconds, the same default as
``reclaim``.

``CLAIMDAG_ACTOR`` pins who the pane claims as. With it unset, the id is
derived from user and host, so the same seat is the same actor across
restarts. ``handles.json`` in the graph directory, or the file named by
``CLAIMDAG_HANDLES``, maps actor ids to names.

The directory is the one in :doc:`Point at a graph <howto-dir>`. The
keys call the verbs in the :doc:`reference <reference>`.
