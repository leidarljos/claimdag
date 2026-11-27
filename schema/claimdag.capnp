@0x88e4bd9dfa52efbf;

# Unpacked snapshot. Hosts mmap work.bin.

struct WorkId {
  hi @0 :UInt64;
  lo @1 :UInt64;
}

enum Status {
  todo @0;
  ready @1;
  claimed @2;
  running @3;
  blocked @4;
  done @5;
  failed @6;
  cancelled @7;
}

enum Role {
  unset @0;
  explore @1;
  architect @2;
  implementor @3;
  verifier @4;
  orchestrator @5;
  general @6;
}

enum Kind {
  unset @0;
  goal @1;
  step @2;
  task @3;
  molecule @4;
}

# Why an edge orders two nodes. hard is ordinal 0, so a missing value is hard.
# needs, pin and history block. advisory is recorded and orders nothing.
enum EdgeKind {
  hard @0;
  needs @1;
  pin @2;
  history @3;
  advisory @4;
}

struct Node {
  id @0 :WorkId;
  kind @1 :Kind;
  status @2 :Status;
  role @3 :Role;
  assignee @4 :WorkId;
  parent @5 :WorkId;
  deps @6 :List(WorkId);
  casGen @7 :UInt64;
  createdUnix @8 :UInt64;
  updatedUnix @9 :UInt64;
  finishedUnix @10 :UInt64;
  summary @11 :Text;
  archived @12 :Bool;
  # Parallel to deps. Absent, or shorter than deps, means hard for each
  # missing slot. A new field: deps stays @6.
  depKinds @13 :List(EdgeKind);
}

struct Snap {
  format @0 :Text;
  nextSeq @1 :UInt64;
  mintSeq @2 :UInt64;
  nodes @3 :List(Node);
}
