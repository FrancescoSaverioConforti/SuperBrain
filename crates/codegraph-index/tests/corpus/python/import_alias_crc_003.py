# Golden corpus fixture — pins CRC-003 (import-alias blindness).
# `resolve_calls` matches a callee by literal name only; it never
# consults `ImportFact.alias`, so a call through an aliased import
# name produces NO edge even though the real target exists and the
# alias mapping is captured correctly by `ImportFact`. Minimized from
# the real pyjanitor repro (`from .ml import get_features_targets as
# _get_features_targets`, called as `_get_features_targets(...)`).
#
# This fixture deliberately does NOT assert a resolved Calls edge
# (there should be none, per the known gap) — it exists so a future
# fix for CRC-003 has an exact, minimal, already-wired-up corpus case
# to flip from "no edge" to "edge, with alias evidence" once fixed.
from helper import real_target as aliased_name


def caller():
    return aliased_name()
