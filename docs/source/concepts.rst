A claimdag graph is a set of nodes plus two kinds of edge. The
:doc:`tutorial <getting-started>` builds the picture below. The
:doc:`explanation <explanation>` is why the generation moves.

.. image:: _static/concepts.svg
   :width: 100.0%

Node
====

A node has a 32-hex id, a kind, a status, a role, an assignee, a parent, a
list of dependencies, a generation, three unix timestamps, a summary, and
an archive flag.

The id is xxHash3-128, printed as 32 lowercase hex characters. The zero id
means unset. ``upsert`` with no ``--id`` mints an id from the node's fields and
a clock salt, so two runs with the same summary mint different ids.

Summary is the open text field. Kind, status, and role are closed enums.

Kinds are ``unset``, ``goal``, ``step``, ``task``, and ``molecule``. The command
line default is ``task``.

Roles are ``unset``, ``explore``, ``architect``, ``implementor``, ``verifier``,
``orchestrator``, and ``general``. The command line default is ``unset``.
``ready --role`` and ``claim-next --role`` use the role when several nodes are
ready. See :doc:`Take the next node <howto-take>`.

Edges
=====

The parent edge is one id on the node. It may be zero. The pane draws this
edge as a forest: a node hangs under its parent. Setting ``--parent`` on
``upsert`` writes it. A parent edge that would make a node its own ancestor
is refused.

The dependency edge is a list. ``link PARENT CHILD`` appends ``PARENT`` to the
child's dependencies. The child stays out of the ready set until every
dependency is ``done``. ``failed`` and ``cancelled`` are terminal, and they
leave the child at ``todo``. ``unlink`` drops one edge. A missing edge is left as
it is. An edge that would close a cycle is refused.

The two edges are independent. The tutorial sets both, which is the usual
shape: the child hangs under the parent, and the child also waits for the
parent to finish.

Status
======

============= ================================================================================================
Status        Meaning
============= ================================================================================================
``todo``      Stored status. Dependencies still decide whether it is ready.
``ready``     Derived. Every dependency is ``done``, and the node is unheld.
``claimed``   One assignee holds it. ``claim`` is the call that writes this.
``running``   Held. ``WorkGraph::set_running`` writes it. The command line has no verb for it.
``blocked``   ``upsert`` may store this on an unheld node. It becomes ready when every dependency is ``done``.
``done``      Terminal. ``complete --status done`` is refused while a dependency is unfinished.
``failed``    Terminal. Allowed while a dependency is unfinished.
``cancelled`` Terminal. Allowed while a dependency is unfinished.
============= ================================================================================================

``ready`` is derived. Asking ``upsert`` for ``ready`` still leaves the
dependencies in charge: a node with an unfinished dependency is stored as
``todo``. A claim requires ``ready``. An assignee holds at most one node that
is ``claimed`` or ``running``.

Terminal status stays until ``reopen``, which returns the node to ready and
moves the generation. The snapshot keeps no record of the earlier
completion. ``archive`` sets a flag on a terminal node and moves the
generation. The status field stays ``done``, ``failed``, or ``cancelled``. The
node stays in the snapshot. ``list`` hides archived nodes unless ``--archived``
or ``--all`` is set.

Generation
==========

The generation is an integer on the node. ``claim`` increments it and prints
the new value. ``reclaim``, ``release``, ``reopen``, ``archive``, and ``unarchive``
increment it. ``renew`` leaves it and updates the timestamp. ``complete``
leaves it.

``complete --gen N`` runs only when the stored generation is still ``N``. The
:doc:`lease how-to <howto-lease>` is the sequence, and the
:doc:`explanation <explanation>` is why the check is optional when
``--gen`` is omitted.

Snapshot
========

The graph directory holds ``work.bin``, an unpacked Cap'n Proto message.
Writers replace the file by rename. Readers map it. The
:doc:`architecture <architecture>` page is the cap and the lock. The
:doc:`directory how-to <howto-dir>` is which path that file is on.
