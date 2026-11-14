// CommonJS Extraction Checkpoint (B2-ID-015): Foo.prototype.X =
// function(){} must be extracted as a Method AND owned (Contains)
// by its real constructor, when the owner is uniquely determinable.
// Minimized from the real request/request B2-EVAL-010 finding
// (request.js's own Request.prototype.X idiom, 27 real instances).
function Request(options) {
  this.options = options;
}

Request.prototype.init = function (options) {
  return options;
};

Request.prototype.start = function () {
  return this.init(this.options);
};
