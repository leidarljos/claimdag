The items below are the public surface of the ``claimdag`` library, read from
the crate sources. ``cargo doc --no-deps -p claimdag`` writes the same items
as HTML. ``docs/build.sh`` and the docs-site workflow copy that HTML to
``rustdoc/claimdag/index.html`` next to this page.

.. raw:: html

   <p><a href="rustdoc/claimdag/index.html">claimdag rustdoc</a></p>

The command line is the :doc:`reference <reference>`. The panes and the
server are the :doc:`how-to guides <howto>`.

Crate
=====

claimdag: CAS claim/complete on a DAG.

The host process is the sole mutator. Snapshot is unpacked Cap'n ``work.bin`` (mmap).

Re-exports
==========

Each name is what a downstream crate writes after ``claimdag::``.

role\ :sub:`affinitytier`
-------------------------

.. code:: rust

   #[must_use]
   pub fn role_affinity_tier(node_role: WorkRole, worker_role: WorkRole) -> u8

Affinity tier of a work node to a worker's role:

-  0: Exact match (node.role == worker\ :sub:`role`), or both are Unset/General
-  1: Flexible match (node.role is Unset/General, or worker\ :sub:`role` is Unset/General)
-  2: Mismatched specialization (e.g. Explorer assigned to Verifier when alternatives exist)

Absent
------

.. code:: rust

   #[derive(Debug, Clone, PartialEq, Eq)]
   pub enum Absent

Why a seat's work graph could not be read.

Variants:

-  ``NoDirectory``: No directory at all: this seat has never had a graph here.
-  ``NoSnapshot``: A directory with no snapshot in it.
-  ``Unreadable``: A snapshot that could not be parsed, and the reason given.

WorkFields
----------

.. code:: rust

   #[derive(Debug, Clone, Copy)]
   pub struct WorkFields<'a>

The fields an upsert sets on a node.

Fields:

-  ``pub kind: WorkKind``: What sort of work it is.
-  ``pub status: WorkStatus``: The status asked for. Readiness is still derived from the dependencies.
-  ``pub role: WorkRole``: Who the work is for.
-  ``pub parent: WorkId``: The node this one hangs under, or zero.
-  ``pub actor: WorkId``: Who is asking, for the ledger.
-  ``pub summary: &'a str``: The one open text field.

WorkGraph
---------

.. code:: rust

   #[derive(Debug, Default)]
   pub struct WorkGraph

WorkGraph::now
~~~~~~~~~~~~~~

.. code:: rust

   pub fn now() -> u64

WorkGraph::mint\ :sub:`id`
~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn mint_id(&mut self, kind: WorkKind, parent: WorkId, summary: &str) -> WorkId

Mint a new work id (xxh3 family). Deterministic from seq + salt.

WorkGraph::get
~~~~~~~~~~~~~~

.. code:: rust

   pub fn get(&self, id: WorkId) -> Option<&WorkNode>

WorkGraph::pruned
~~~~~~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn pruned(&self, id: WorkId) -> Option<WorkStatus>

The status a node finished with before prune dropped it. ``None`` for a node the graph holds, never had, or pruned too long ago to remember.

WorkGraph::list
~~~~~~~~~~~~~~~

.. code:: rust

   pub fn list(&self) -> Vec<&WorkNode>

WorkGraph::list\ :sub:`view`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn list_view(&self, show_terminal: bool, show_archived: bool) -> Vec<&WorkNode>

Default live view hides terminal and archived nodes.

WorkGraph::warnings
~~~~~~~~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn warnings(&self) -> &[String]

Warnings recorded on this graph since it was loaded, oldest first.

WorkGraph::upsert
~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn upsert(&mut self, mut id: WorkId, fields: WorkFields<'_>) -> Result<WorkId, String>

What an upsert is asserting about a node.

WorkGraph::link\ :sub:`dep`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn link_dep(&mut self, parent: WorkId, child: WorkId, actor: WorkId) -> Result<(), String>

WorkGraph::unlink\ :sub:`dep`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn unlink_dep(
           &mut self,
           parent: WorkId,
           child: WorkId,
           actor: WorkId,
       ) -> Result<(), String>

Drop a boolean hard dep. Missing edge is Ok.

WorkGraph::critical\ :sub:`depth`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn critical_depth(&self) -> HashMap<WorkId, u32>

The longest chain of unfinished work below each node, unit cost: the list-scheduling priority (Hu, doi:10.1287/opre.9.6.841; Graham, doi:10.1137/0117039). Finished and archived nodes count for nothing.

WorkGraph::ready\ :sub:`view`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn ready_view(&self) -> Vec<&WorkNode>

The nodes a seat may take right now: unblocked and unfinished, the predicate ``claim`` enforces. Deepest chain first, then most recently touched, then id.

WorkGraph::ready\ :sub:`viewbalanced`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn ready_view_balanced(
           &self,
           role: WorkRole,
           assignee: WorkId,
           slack: usize,
       ) -> Vec<&WorkNode>

Ready nodes ordered by balanced suitability for ``(role, assignee)``:

-  1. Critical depth slack bands: candidates within ``slack`` of the maximum critical depth form Band 0, preserving critical-path progress.
-  2. Role affinity: exact match (tier 0) before general/unset (tier 1) before mismatched specialization (tier 2).
-  3. Worker dispersion / Power-of-Two choices: pseudo-random hash ranking seeded by worker identity so concurrent workers naturally inspect distinct candidates without central lock contention.
-  4. Starvation prevention: older ready nodes (updated\ :sub:`unix`) are prioritized.
-  5. Deterministic tie-break by node id.

WorkGraph::select\ :sub:`balancedready`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn select_balanced_ready(
           &self,
           role: WorkRole,
           assignee: WorkId,
           slack: usize,
       ) -> Option<&WorkNode>

Pick the single best candidate ready node for ``(role, assignee)`` using Mitzenmacher's Power of Two Random Choices over the top critical-slack candidate pool.

WorkGraph::claim
~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn claim(
           &mut self,
           id: WorkId,
           assignee: WorkId,
           expected_gen: Option<u64>,
       ) -> Result<u64, String>

WorkGraph::claim\ :sub:`next`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn claim_next(
           &mut self,
           assignee: WorkId,
           role: WorkRole,
           slack: usize,
       ) -> Result<(WorkId, u64), String>

Atomically find and claim the best ready work node for ``assignee`` matching ``role`` within critical path ``slack``.

Avoids CAS stampedes and two-phase check-then-act race conditions by combining candidate selection and state transition into one atomic step under the graph lock.

WorkGraph::reclaim
~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn reclaim(&mut self, lease_secs: u64) -> Vec<WorkId>

Return every claim quiet for longer than ``lease_secs`` to ``Ready``, bumping the generation so a holder that wakes later is fenced. Returns the nodes handed back, oldest first.

WorkGraph::release
~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn release(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String>

Hand one claim back on purpose. The node returns to ``ready``, the assignee clears, and the generation moves, exactly as a reclaim does: the holder chose to stop before the lease ran out. A zero actor still hands it back and records ``no-actor``. A different actor still hands it back and records ``not-assignee``. ``WorkGraph::release_forced`` is the explicit way to hand back a claim for nobody without that warning.

A node prune dropped after it finished is held by nobody already, so releasing it succeeds, changes nothing, and answers generation 0; ``WorkGraph::pruned`` says which status it finished with.

WorkGraph::release\ :sub:`forced`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn release_forced(&mut self, id: WorkId) -> Result<(u64, WorkId), String>

Hand a claim back whoever holds it. The ledger records the op as ``release-forced``. Returns the new generation and the holder it was taken from, so the caller can log both.

WorkGraph::reopen
~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn reopen(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String>

Bring a terminal node back to ``ready``, generation moved, so work on the same tracker id can be taken again: a new sitting on old work. The snapshot keeps no history of operations, so nothing on disk records the earlier completion; see ``WorkLedgerEntry``.

WorkGraph::renew
~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn renew(&mut self, id: WorkId, actor: WorkId) -> Result<u64, String>

Move the lease forward. The generation stays: a renewal keeps the same owner.

WorkGraph::set\ :sub:`running`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn set_running(&mut self, id: WorkId, actor: WorkId) -> Result<(), String>

WorkGraph::complete
~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn complete(
           &mut self,
           id: WorkId,
           status: WorkStatus,
           summary: &str,
           actor: WorkId,
           expected_gen: Option<u64>,
       ) -> Result<(), String>

Finish a node. ``expected_gen`` is the fencing token from the claim: the generation moves on every claim and reclaim, so a stale holder is refused. ``None`` skips the check. A zero actor still finishes the node and records ``no-actor``. A different actor still finishes it and records ``not-assignee``. ``WorkGraph::complete_forced`` is the explicit way to finish a node for nobody without that warning.

A node prune dropped after it finished is as finished as it gets, so completing it again succeeds and changes nothing, as it does for a terminal node still in the graph; ``WorkGraph::pruned`` says which status it finished with.

WorkGraph::complete\ :sub:`forced`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn complete_forced(
           &mut self,
           id: WorkId,
           status: WorkStatus,
           summary: &str,
           expected_gen: Option<u64>,
       ) -> Result<WorkId, String>

Finish a node whoever holds it. Dependencies and ``expected_gen`` are still checked. The ledger records the op as ``complete-forced``. Returns the holder the node had, so the caller can log it.

WorkGraph::archive
~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn archive(&mut self, id: WorkId, actor: WorkId) -> Result<(), String>

WorkGraph::unarchive
~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn unarchive(&mut self, id: WorkId, actor: WorkId) -> Result<(), String>

WorkGraph::verify
~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn verify(&self) -> Result<(), String>

WorkGraph::open\ :sub:`dir`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn open_dir(dir: &Path) -> Result<Self, Absent>

The graph in a directory, or why there is none: no directory, an unreadable snapshot, or an empty graph. Readers need the difference.

WorkGraph::load\ :sub:`forwrite`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn load_for_write(dir: &Path) -> Result<Self, Absent>

The graph a writer starts from: empty when the seat has none yet, and refused when a snapshot is there but cannot be read.

``WorkGraph::load_dir`` answers an unreadable snapshot with an empty graph, so the next save replaces every node with the one being written. A torn or truncated ``work.bin`` has to stop the writer instead.

**Errors**

``Absent::Unreadable`` when ``work.bin`` exists and does not parse.

WorkGraph::load\ :sub:`dir`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn load_dir(dir: &Path) -> Self

The graph in a directory, empty when there is none or it cannot be read. A writer wants ``WorkGraph::load_for_write``, a reader ``WorkGraph::open_dir``.

WorkGraph::save\ :sub:`dir`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn save_dir(&self, dir: &Path) -> Result<(), String>

Atomic replace of ``$dir/work.bin`` (unpacked Cap'n), then the nodes prune dropped onto ``pruned.log``.

WorkKind
--------

.. code:: rust

   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum WorkKind

Variants:

-  ``Unset``
-  ``Goal``
-  ``Step``
-  ``Task``
-  ``Molecule``

WorkKind::as\ :sub:`str`
~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn as_str(self) -> &'static str

WorkKind::parse\ :sub:`str`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn parse_str(s: &str) -> Option<Self>

WorkLedgerEntry
---------------

.. code:: rust

   #[derive(Debug, Clone)]
   pub struct WorkLedgerEntry

One operation, as the graph saw it in this process.

The ledger lives in memory only. ``work.bin`` does not store it, so a command-line call starts with an empty ledger and its entries are gone when it exits. It is the list of operations this process has applied.

Fields:

-  ``pub seq: u64``
-  ``pub ts_unix: u64``
-  ``pub work_id: WorkId``
-  ``pub actor: WorkId``
-  ``pub op: &'static str``: Closed op tag (static str interned as &'static str in practice).

WorkNode
--------

.. code:: rust

   #[derive(Debug, Clone)]
   pub struct WorkNode

Fields:

-  ``pub id: WorkId``
-  ``pub kind: WorkKind``
-  ``pub status: WorkStatus``
-  ``pub role: WorkRole``
-  ``pub assignee: WorkId``
-  ``pub parent: WorkId``
-  ``pub deps: Vec<WorkId>``
-  ``pub cas_gen: u64``
-  ``pub created_unix: u64``
-  ``pub updated_unix: u64``
-  ``pub finished_unix: u64``
-  ``pub summary: String``: Sole open-content field (writer-owned length).
-  ``pub archived: bool``: Soft hide. Only legal on a terminal node.

WorkRole
--------

.. code:: rust

   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum WorkRole

Variants:

-  ``Unset``
-  ``Explore``
-  ``Architect``
-  ``Implementor``
-  ``Verifier``
-  ``Orchestrator``
-  ``General``

WorkRole::as\ :sub:`str`
~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn as_str(self) -> &'static str

WorkRole::parse\ :sub:`str`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn parse_str(s: &str) -> Option<Self>

WorkStatus
----------

.. code:: rust

   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum WorkStatus

Work node status (CAS claim/complete).

Variants:

-  ``Todo``
-  ``Ready``
-  ``Claimed``
-  ``Running``
-  ``Blocked``
-  ``Done``
-  ``Failed``
-  ``Cancelled``

WorkStatus::as\ :sub:`str`
~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn as_str(self) -> &'static str

WorkStatus::parse\ :sub:`str`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn parse_str(s: &str) -> Option<Self>

WorkStatus::is\ :sub:`terminal`
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn is_terminal(self) -> bool

SNAP\ :sub:`FILE`
-----------------

.. code:: rust

   pub const SNAP_FILE: &str = crate::snap::SNAP_BIN

Snapshot filename under the host state dir (unpacked Cap'n, mmap).

mint\ :sub:`workid`
-------------------

.. code:: rust

   pub fn mint_work_id(
       parent: WorkId,
       root: WorkId,
       role: &str,
       summary: &str,
       seq: u64,
       salt: &[u8],
   ) -> WorkId

Same length-prefixed xxh3 family the seat already minted with.

WorkId
------

.. code:: rust

   #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
   pub struct WorkId

Wire id is two u64 halves (xxHash3-128). Zero = unset / mint.

Fields:

-  ``pub hi: u64``
-  ``pub lo: u64``

WorkId::is\ :sub:`zero`
~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn is_zero(self) -> bool

WorkId::to\ :sub:`hex`
~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn to_hex(self) -> String

32 lowercase hex chars. Off-disk / CLI only.

WorkId::from\ :sub:`hex`
~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn from_hex(s: &str) -> Option<Self>

WorkId::to\ :sub:`bebytes`
~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn to_be_bytes(self) -> [u8

WorkId::from\ :sub:`u128`
~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn from_u128(v: u128) -> Self

WorkId::to\ :sub:`u128`
~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn to_u128(self) -> u128

append\ :sub:`warn`
-------------------

Source name: ``label::append``.

.. code:: rust

   pub fn append(dir: &Path, text: &str)

Append ``text`` to ``warn.log``. A directory that cannot take the file is skipped: the caller has already shown the line where a seat reads it.

emit\ :sub:`warn`
-----------------

Source name: ``label::emit``.

.. code:: rust

   pub fn emit(dir: &Path, text: &str)

Show ``text`` on stderr and append it to ``warn.log``.

warn\ :sub:`line`
-----------------

Source name: ``label::line``.

.. code:: rust

   #[must_use]
   pub fn line(rule: &str, detail: &str) -> String

One warning a seat can read.

WARN\ :sub:`LOG`
----------------

Source name: ``label::FILE``.

.. code:: rust

   pub const FILE: &str = "warn.log"

The file beside ``work.bin`` that keeps these lines, oldest first.

locate
------

.. code:: rust

   #[must_use]
   pub fn locate(explicit: Option<PathBuf>) -> Located

Every location this seat would look in, and the one it uses.

resolve\ :sub:`dir`
-------------------

.. code:: rust

   #[must_use]
   pub fn resolve_dir(explicit: Option<PathBuf>) -> PathBuf

Where the work graph lives when the caller does not say.

``CLAIMDAG_DIR``, then ``$XDG_RUNTIME_DIR/claimdag``. Without either the defaults decide: ``$XDG_STATE_HOME/claimdag``, ``~/.local/state/claimdag``, ``/tmp/claimdag-UID`` and the old ``/tmp/claimdag``, tried in that order. The first that holds a graph this user owns is used, else the first that applies, so a seat whose environment lacks one of them still reads the graph another seat started in a later one. Every default is this user's own: a shared ``/tmp/claimdag`` made one user's first claim fail on another user's directory. The defaults are ``$XDG_STATE_HOME/claimdag``, ``~/.local/state/claimdag``, ``/tmp/claimdag-UID``, and ``/tmp/claimdag``, one set per seat, so two users on one host keep separate graphs. The working directory stays off that list, and ``work.bin`` stays in the seat directory.

Unless ``--dir`` or ``CLAIMDAG_DIR`` named the directory, says on stderr which graph it uses when another location holds one too, once.

Located
-------

.. code:: rust

   #[derive(Debug, Clone)]
   pub struct Located

Every location a seat would look in, in the order it tries them, and the one it uses. Printed, one line per location: the directory, whether it is in use, holds work or has no graph, and the rule that named it.

Located::dir
~~~~~~~~~~~~

.. code:: rust

   #[must_use]
   pub fn dir(&self) -> &Path

The graph directory in use.

Located::move\ :sub:`graph`
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code:: rust

   pub fn move_graph(&self) -> Result<Option<(PathBuf, PathBuf)>, String>

Move the graph in use into the first default, when the defaults picked a later one because only it held a graph. Returns where it was and where it is, or ``None`` when there was nothing to move.

**Errors**

Fails when either directory cannot be made or locked, or a file in it cannot be moved.

DEFAULT\ :sub:`LEASESECS`
-------------------------

.. code:: rust

   pub const DEFAULT_LEASE_SECS: u64 = 900

How long a claim stands before anybody may take it back, in seconds. One value for every seat, or one takes back what another still holds.

A sitting does not renew while its holder still runs, so this is how long a gone holder may stay quiet before the next sitting hands the claim back. On an Intel Xeon (family 6, model 207, 8 cores), at e0f79d7, five runs of ``ljos sitting ISSUE --verbose`` (ljos 0.37.0, vissue 0.23.1, deedar 0.6.1, this claimdag, ``PACKSET_URL=off``, ``LJOS_TRACKER_GIT=off``) put the longest section at ``== doctor``, 242.9 ms median (242.2 to 243.1), and the whole sitting at 541 ms. One sitting on a graph of 4096 nodes took 162 ms to list that graph and reclaim, still under the doctor. Fifteen minutes is many of those sittings, so one that is still printing stays inside the lease. A crashed holder gives the node back the same day, and gives back the one claim its assignee is allowed. The measured step is under this, so the value stays 900.

SNAP\ :sub:`BIN`
----------------

.. code:: rust

   pub const SNAP_BIN: &str = "work.bin"

lock\ :sub:`dir`
----------------

.. code:: rust

   pub fn lock_dir(dir: &Path) -> Result<Lock, String>

Take the exclusive lock on ``dir/lock``, creating the directory, and write this process's pid into it. Waits a bounded time while another process holds it. A default location is made private. One another user owns is still locked, and the warning from ``prepare`` is recorded.

**Errors**

Fails when the directory or the lock file cannot be created, the lock cannot be taken or is still held when the wait runs out (the one line names the holder's pid and how to release it). A default location another user owns is still locked, and ``prepare`` records ``not-owner``.

Module ``lock``
===============

One mutator at a time across processes: an advisory lock on the graph directory, held from load to save.

lock::Lock
----------

Source name: ``lock::Lock``.

.. code:: rust

   pub struct Lock

The lock, released on drop. While it is held the file holds this process's pid, so a writer left waiting can name it.

lock::lock\ :sub:`dir`
----------------------

Source name: ``lock::lock_dir``.

.. code:: rust

   pub fn lock_dir(dir: &Path) -> Result<Lock, String>

Take the exclusive lock on ``dir/lock``, creating the directory, and write this process's pid into it. Waits a bounded time while another process holds it. A default location is made private. One another user owns is still locked, and the warning from ``prepare`` is recorded.

**Errors**

Fails when the directory or the lock file cannot be created, the lock cannot be taken or is still held when the wait runs out (the one line names the holder's pid and how to release it). A default location another user owns is still locked, and ``prepare`` records ``not-owner``.

lock::prepare
-------------

Source name: ``lock::prepare``.

.. code:: rust

   pub fn prepare(dir: &Path, default: bool, uid: u32) -> Result<Option<String>, String>

Make the graph directory. A default location (the state directory, ``/tmp/claimdag-UID`` or the old ``/tmp/claimdag``) is made with mode 0700, so another user cannot read a graph this seat created under ``/tmp``. A default another user already owns is still used: the returned line is ``not-owner``, and the caller records it. A directory named by ``CLAIMDAG_DIR`` or ``--dir`` is used as given, with no such line: a team can share one on purpose.

**Errors**

Fails when the directory cannot be made or its owner cannot be read.

lock::owned\ :sub:`by`
----------------------

Source name: ``lock::owned_by``.

.. code:: rust

   pub fn owned_by(dir: &Path, uid: u32) -> Result<Option<String>, String>

The warning when a default graph directory belongs to someone else.

A shared default made one user's claims land in another user's graph. The directory is still the one to open: the line names the owner, and the call proceeds. ``Ok(None)`` when ``uid`` owns it.

**Errors**

Fails when the directory's owner cannot be read.
