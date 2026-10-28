---
schema: 1
id: d23jz
type: decision
title: >-
  A3 Task 6 (PiiStore + sensitive-write pipeline) hit a Critical,
  repro-confirmed data-loss bug: commit_staged overwrote the only copy of a
  sensitive note's prior ciphertext before the pointer rename, so a crash
  mid-update could permanently lose committed content — fix in progress, not yet
  verified
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-28T12:09:07.429Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Independent reviewer (opus-tier) wrote and ran an actual repro proving: crash at AfterPiiCommittedBeforePointerRename on an update left the v1 Markdown pointer reading back v2 ciphertext, and recovery's fix_intent_recorded then deleted that ciphertext entirely, losing v1's committed content permanently. This is the exact two-store-drift failure mode the user's plan-revision round explicitly required this task to prevent. Also found: EncryptedStoreAvailability hardcoded to Available in propose.rs (defeats Task 4's design — a broken store now blocks even non-sensitive writes instead of only rejecting Sensitive notes), DeletionLog::record_deletion running outside the journaled operation (stale-version tombstone can mark deleted while content stays live), and a retry-matching gap. Fix dispatched: a PII before-image mechanism mirroring the existing Markdown one, plus a new crash test at the exact failure point. Not yet re-reviewed as of this record.
