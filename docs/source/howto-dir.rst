.. _point-at-a-graph-1:

Point at a graph
================

``--dir DIR`` names the directory. If that flag is absent, ``CLAIMDAG_DIR`` names
it. If both are absent, the directory is ``$XDG_RUNTIME_DIR/claimdag`` when
that variable is set.

With no runtime directory the defaults are tried in this order:
``$XDG_STATE_HOME/claimdag``, ``~/.local/state/claimdag``, ``/tmp/claimdag-UID``,
and ``/tmp/claimdag``. The first that holds a graph this user owns is used,
otherwise the first that applies. ``/tmp`` here is the path ``/tmp``, so two
seats on one host reach the same ``/tmp/claimdag-UID``. The working directory
is never a default.

Each default belongs to one user. claimdag creates it with mode 0700. A
default another user owns is still opened. The call writes ``allow``, ``warn``,
and ``not-owner``, separated by tabs, on stderr and in ``warn.log``. A directory
named by ``CLAIMDAG_DIR`` or ``--dir`` is used as given, with no such line, so
a team can share one on purpose.

A reader that finds no graph says so. The message names the directory and
says it holds no ``work.bin``, or that the directory does not exist. A writer
on a missing directory creates the snapshot. The first ``upsert`` is that
writer:

.. code:: console

   $ claimdag --dir /tmp/demo-claims upsert --summary "land the adapter"
   092dca29...

``claimdag where`` prints every location in the order above. Each line is the
directory, then ``in use``, ``holds work``, or ``no graph``, then the rule that
named it: ``--dir``, ``CLAIMDAG_DIR``, ``XDG_RUNTIME_DIR``, ``XDG_STATE_HOME``,
``HOME``, ``uid``, or ``old default``. When the graph in use is a later default,
the last line says ``claimdag where --move`` will move it to the first
default. ``--move`` moves ``work.bin``, ``forced.log``, ``warn.log``, and
``handles.json``.

When another location also holds a graph, the command line, the pane, and
the server write one stderr line: which directory is in use, and which
others hold work. They write it once, and again when that list changes. A
directory named by ``CLAIMDAG_DIR`` or ``--dir`` gets no such line. The line
is kept beside the snapshot in a file named ``elsewhere``.

A seat inside a login session uses the runtime directory. A seat outside it,
such as cron, ``su``, or a container, uses the defaults. Set the same
``CLAIMDAG_DIR`` in each to give both one graph.

The :doc:`reference <reference>` lists the variables. The
:doc:`pane <howto-pane>` and the :doc:`server <howto-mcp>` use this
same directory.
