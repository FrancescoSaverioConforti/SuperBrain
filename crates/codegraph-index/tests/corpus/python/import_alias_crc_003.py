# Golden corpus fixture — pins CRC-003 (import-alias blindness).
# Originally documented a known gap: the pre-A4 `resolve_calls` matched
# a callee by literal name only and never consulted `ImportFact.alias`,
# so a call through an aliased import name produced NO edge even
# though the real target existed and the alias mapping was captured
# correctly by `ImportFact`. Minimized from the real pyjanitor repro
# (`from .ml import get_features_targets as _get_features_targets`,
# called as `_get_features_targets(...)`).
#
# CALL-RES-A4's import-alias mechanism, now wired into the real
# indexer, resolves this clean (collision-free) alias call to a real
# Calls edge pointing at helper.py's real_target.
from helper import real_target as aliased_name


def caller():
    return aliased_name()
