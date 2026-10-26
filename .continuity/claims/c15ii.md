---
schema: 1
id: c15ii
type: constraint
title: >-
  I-C: Vault's symlink-escape check (record_path_checked) canonicalizes the bare
  root/id path but all actual I/O goes to root/id.record — the check guards a
  path that's never read or written, providing no real protection against a
  symlink planted at the .record path itself
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T18:06:06.114Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Found on 3rd whole-branch review, reproduced empirically on Windows: a symlink at root/note-a.record pointing outside the vault lets read_note and propose_and_commit's version check both read/operate on content OUTSIDE the vault root undetected. Task 6's own dedicated path-safety review and two subsequent whole-branch reviews all missed this because the existing test plants the symlink at the bare id path (root/escape), not at the actual .record path, so it passes without proving anything. Low exploitability under A1's single-process non-adversarial threat model (same class as the already-accepted TOCTOU gap [c12ah]), hence Important not Critical. Fix is ~10 lines: check symlink_metadata on the candidate .record path itself, not just its parent directory's canonicalization. Alternative: document as accepted limitation L5 and correct evidence.md's §4.1/§4.2 PASS claims, which currently overclaim this is closed.
