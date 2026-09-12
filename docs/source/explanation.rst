===========
Explanation
===========



A claim is a lease
------------------

A claim with no expiry is a claim a crashed worker keeps. The node stays
claimed with nobody on it, and because one assignee has at most one live
node, that worker's identity can never claim again either. One process
dying takes a node and an identity with it for good. So a claim is a lease:
``renew`` says the holder is alive, and going quiet past the lease returns the
node to ready for somebody else. Chubby's leases and ZooKeeper's ephemeral
nodes answer the same question the same way; the graph has one mutator and
no way to ask whether a worker is alive, so the only evidence is whether it
said so recently.

A generation is a fence
-----------------------

The generation moves on every claim and every reclaim. A holder that
stalled long enough to be reclaimed wakes up as if it still owned the
node, and an assignee check alone does not stop it: a reclaim clears the
assignee, and a cleared assignee is what lets anybody finish an unheld node.
A holder that passes the generation it was given is refused exactly when
the world moved underneath it. ``renew`` leaves the generation alone, because
a renewal is not a change of ownership and moving it would invalidate the
token the holder is about to complete with.

Readiness is derived
--------------------

A caller may ask for ``ready`` and the dependencies decide. If status could be
asserted, it would lie to every reader who picks work from it, and the
refusal at claim time would not help the one who believed it. The parent
chain is checked for loops the same way a dependency edge is, so a node
cannot be its own ancestor and a walk up the tree always ends.

Which ready node
----------------

When several nodes are ready, the one with the longest chain of work
waiting below it is the one to take first: finishing it frees the most.
That is the critical-path rule for scheduling a partial order on identical
machines (Hu, https://doi.org/10.1287/opre.9.6.841; Graham, https://doi.org/10.1137/0117039), and
the heterogeneous earliest-finish-time heuristic (HEFT) extends it to
workers of different speeds (https://doi.org/10.1109/71.993206). Computing the depth below every node is one pass
over the graph and costs under a millisecond at four thousand nodes.

Outside the join
----------------

The tracker knows what the work is and who agrees. The deed store knows
what the work produced. The pack knows what was learned. claimdag knows
only which session node is held and by whom, and it is deliberately not
joined to the others by the accession: a session's scheduling state should
not outlive the session or leak into what is cited. Completing a node here
does not close a ticket. The seat maps tracker ids onto nodes for
convenience, and the mapping lives in the seat.
