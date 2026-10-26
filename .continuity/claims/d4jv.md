---
schema: 1
id: d4jv
type: decision
title: >-
  SuperBrain is private/personal, not open source — crate Cargo.toml license
  fields are a placeholder to satisfy cargo-deny, not a real public license
  grant; no LICENSE file is published
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-10-26T14:06:23.172Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

User confirmed when asked, after Task 1's implementer found no LICENSE file or declared license anywhere in the repo. license = "Apache-2.0" is kept in Cargo.toml only to satisfy cargo-deny's unlicensed-crate check and workspace.package.publish = false marks every crate explicitly non-publishable to crates.io.
