// CommonJS Extraction Checkpoint (B2-ID-014): exports.X = function(){}
// and module.exports.X = function(){} must both be extracted as real
// Function candidates. Minimized from the real request/request
// B2-EVAL-010 finding (lib/cookies.js's exports.jar/exports.parse,
// each with real cross-file call sites like request.js's `cookies.jar()`).
exports.jar = function (store) {
  return store;
};

module.exports.parse = function (str) {
  return str.trim();
};
