========
claimdag
========


.. raw:: html

   <div class="vi-hero">
     <div class="vi-hero-brand">
       <img class="vi-hero-mark" src="_static/mark.svg" width="64" height="64" alt="" />
       <div>
         <p class="vi-hero-name">claimdag</p>
         <p class="vi-hero-tag">What is claimable right now?</p>
       </div>
     </div>
     <p class="vi-hero-tagline">Compare-and-swap claims over a session's work graph, with leases.</p>
     <div class="vi-hero-pills">
       <span>One mutator</span>
       <span>Leases, not locks</span>
       <span>CLI + TUI + MCP</span>
     </div>
     <div class="vi-hero-actions">
       <a class="vi-btn vi-btn-gold" href="getting-started.html">Get started</a>
       <a class="vi-btn vi-btn-ghost" href="reference.html">Reference</a>
     </div>
   </div>

A session hands out work. Two or more workers ask the same question at once:
what is unblocked and unheld? claimdag answers from a small graph of nodes
with hard dependencies. A claim is compare-and-swap on the node's
generation, so two workers cannot both take it. A claim is a lease, so a
worker that dies gives the node back. Completing a node here does not close
a ticket anywhere; the tracker is a different store.

Install
-------

.. code:: console

    $ cargo install --git https://github.com/leidarljos/claimdag claimdag-cli
    $ cargo install --git https://github.com/leidarljos/claimdag claimdag-mcp   # optional
    $ cargo install --git https://github.com/leidarljos/claimdag claimdag-tui   # optional pane

The graph lives in ``$XDG_RUNTIME_DIR/claimdag`` unless ``CLAIMDAG_DIR`` or
``--dir`` says otherwise. It is session state and does not outlive the seat.

First minute
------------

.. code:: console

    $ a=$(claimdag upsert --summary "parse the manifest header")
    $ claimdag claim $a --assignee $(printf '%032x' 7)
    gen=2
    $ claimdag list
    092dca29...  claimed  task  00000000  gen=2  parse the manifest header
    $ claimdag complete $a --status done
    092dca29...  done

.. toctree::
   :maxdepth: 1
   :caption: Guides
   :hidden:

   getting-started
   howto
   reference
   explanation
   architecture
