// CommonJS Extraction Checkpoint: Foo.prototype.bar assigned twice (a
// real, if unusual, patch-later shape) -- both assignments must be
// extracted as distinct Method snapshots, both owned by the same real
// Foo constructor, never collapsed or dropped.
function Foo() {}

Foo.prototype.bar = function (x) {
  return x;
};

Foo.prototype.bar = function (x) {
  return x + 1;
};
