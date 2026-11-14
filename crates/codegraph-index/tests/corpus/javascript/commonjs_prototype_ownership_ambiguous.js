// CommonJS Extraction Checkpoint: the real ownership-ambiguous case
// from this checkpoint's own directive -- `Foo` is bound only via
// require() (an imported module reference, never a real Function/
// Class snapshot in this pipeline), so Foo.prototype.bar's own
// ownership resolver must fail closed: no Contains edge invented for
// a receiver that is not a real, same-file Function/Class.
var Foo = require('./some-other-module');

Foo.prototype.bar = function (x) {
  return x;
};
