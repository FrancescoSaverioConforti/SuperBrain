---
schema: 1
id: d59uh
type: decision
title: >-
  User rejected full SD reflash + root-only account plan for Raspberry Pi;
  scoped down to leaving E:\ (boot partition) as-is since it was already
  original, with WiFi/SSH/root setup left for the user to do manually
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-02T15:45:22.113Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Raspberry SD on E:\ has 2 partitions: E:\ is 512MB FAT32 boot (already clean, nothing extra found), second ~63GB partition is ext4 rootfs unreadable from Windows. Any package cleanup the user wants must happen from inside the booted Pi (SSH/console), not from Windows.
