// Golden corpus fixture — pins CRC-002 (scope-blindness false
// positive). `handleSubmit`, as used inside `Child`'s own body, is
// bound by that component's own destructured parameter list — a
// local binding that should shadow the unrelated same-named function
// in `Parent`. The pipeline does not currently recognize this, so
// this fixture deliberately asserts that a (wrong, confidently
// StaticAnalysis-labeled) edge IS currently produced — documenting
// the known gap, not a desired outcome. A future fix for CRC-002
// would need to flip this fixture's own expectation to "no edge."
// Minimized from the real yoonge/conduit-react repro.
function handleSubmit() {
  return "the real, unrelated, top-level function";
}

function Child({ handleSubmit }) {
  handleSubmit();
}
