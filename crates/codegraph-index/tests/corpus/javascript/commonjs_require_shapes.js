// CommonJS Extraction Checkpoint (B2-ID-013): require() -> ImportFact.
// Minimized real shapes: simple binding, aliased binding, destructured
// binding, and a package (non-relative) require.
const helper = require('./helper');
var utils = require('./utils');
const { a, b } = require('./destructured');
const extend = require('extend');

function useThem() {
  return helper.run() + utils.format() + a + b + extend;
}
